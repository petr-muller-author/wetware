//! Thought creation service - persists a new thought and links its entity mentions.
//!
//! Shared by `wet add` and the interactive composer so both take exactly the same
//! path. Like `entity_resolution`, this service touches the database directly.

use crate::errors::ThoughtError;
use crate::models::thought::Thought;
use crate::services::{entity_parser, entity_resolution};
use crate::storage::entities_repository::EntitiesRepository;
use crate::storage::thoughts_repository::ThoughtsRepository;
use chrono::NaiveDate;
use rusqlite::Connection;

/// Persist a thought and link every entity mentioned in its content.
///
/// The whole operation runs in a single transaction: if entity resolution or
/// linking fails, the thought itself is not left behind.
///
/// `date` of `None` timestamps the thought with the current instant (matching
/// `Thought::new`); `Some(date)` pins it to midnight UTC on that date.
///
/// Returns the new thought's ID and the unique entity names extracted from its
/// content, in first-occurrence order. Note that a mention resolving to an
/// ambiguous alias is counted here but deliberately not linked - see
/// [`entity_resolution::resolve_or_create_entity`].
pub fn create_thought(
    conn: &mut Connection,
    content: &str,
    date: Option<NaiveDate>,
) -> Result<(i64, Vec<String>), ThoughtError> {
    let thought = match date {
        Some(date) => {
            let datetime = date
                .and_hms_opt(0, 0, 0)
                .expect("midnight is a valid time for any date")
                .and_utc();
            Thought::new_with_date(content.to_string(), datetime)?
        }
        None => Thought::new(content.to_string())?,
    };

    let tx = conn.transaction()?;

    let thought_id = ThoughtsRepository::save(&tx, &thought)?;

    let entity_names = entity_parser::extract_unique_entities(content);
    for entity_name in &entity_names {
        if let Some(entity_id) = entity_resolution::resolve_or_create_entity(&tx, entity_name)? {
            EntitiesRepository::link_to_thought(&tx, entity_id, thought_id)?;
        }
    }

    tx.commit()?;

    Ok((thought_id, entity_names))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::entity::Entity;
    use crate::storage::connection::get_memory_connection;
    use crate::storage::entity_aliases_repository::EntityAliasesRepository;
    use crate::storage::migrations::run_migrations;

    fn setup() -> Connection {
        let conn = get_memory_connection().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn test_create_thought_without_date_uses_now() {
        let mut conn = setup();
        let before = chrono::Utc::now();

        let (id, entities) = create_thought(&mut conn, "a plain thought", None).unwrap();

        assert!(entities.is_empty());
        let saved = ThoughtsRepository::get_by_id(&conn, id).unwrap();
        assert_eq!(saved.content, "a plain thought");
        assert!(saved.created_at >= before);
    }

    #[test]
    fn test_create_thought_with_date_pins_to_midnight() {
        let mut conn = setup();
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();

        let (id, _) = create_thought(&mut conn, "a backdated thought", Some(date)).unwrap();

        let saved = ThoughtsRepository::get_by_id(&conn, id).unwrap();
        assert_eq!(saved.created_at, date.and_hms_opt(0, 0, 0).unwrap().and_utc());
    }

    #[test]
    fn test_create_thought_links_entity_mentions() {
        let mut conn = setup();

        let (id, entities) = create_thought(&mut conn, "chatted with [Alice] about [Rust]", None).unwrap();

        assert_eq!(entities, vec!["Alice".to_string(), "Rust".to_string()]);
        assert!(EntitiesRepository::find_by_name(&conn, "alice").unwrap().is_some());
        let linked = ThoughtsRepository::list_by_entity(&conn, "Alice").unwrap();
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].id, Some(id));
    }

    #[test]
    fn test_create_thought_resolves_alias_to_existing_entity() {
        let mut conn = setup();
        let alice_id = EntitiesRepository::find_or_create(&conn, &Entity::new("Alice".to_string())).unwrap();
        EntityAliasesRepository::add_alias(&conn, alice_id, "alicia").unwrap();

        create_thought(&mut conn, "paired with [al](alicia)", None).unwrap();

        // The alias resolved to the existing entity; no duplicate "alicia" created.
        assert!(EntitiesRepository::find_by_name(&conn, "alicia").unwrap().is_none());
        assert_eq!(ThoughtsRepository::list_by_entity(&conn, "Alice").unwrap().len(), 1);
    }

    #[test]
    fn test_create_thought_deduplicates_repeated_mentions() {
        let mut conn = setup();

        let (_, entities) = create_thought(&mut conn, "[Alice] and [alice] again", None).unwrap();

        assert_eq!(entities, vec!["Alice".to_string()]);
    }

    #[test]
    fn test_create_thought_rejects_empty_content_without_writing() {
        let mut conn = setup();

        let result = create_thought(&mut conn, "   ", None);

        assert!(matches!(result, Err(ThoughtError::EmptyContent)));
        assert!(ThoughtsRepository::list_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn test_create_thought_rolls_back_when_linking_fails() {
        let mut conn = setup();
        // Drop the junction table so linking fails partway through, after the
        // thought row has already been inserted inside the transaction.
        conn.execute("DROP TABLE thought_entities", []).unwrap();

        let result = create_thought(&mut conn, "mentions [Alice]", None);

        assert!(result.is_err());
        assert!(
            ThoughtsRepository::list_all(&conn).unwrap().is_empty(),
            "the thought should not survive a failed link"
        );
    }
}
