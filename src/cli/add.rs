/// Add command implementation
use crate::errors::ThoughtError;
use crate::services::{date_parser, thought_writer};
use crate::storage::connection::get_connection;
use crate::storage::migrations::run_migrations;
use std::path::Path;

/// Execute the add command
///
/// # Arguments
/// * `content` - Text of the thought, including any `[Entity]` mentions
/// * `date` - Optional date string; see [`date_parser`] for the accepted forms
/// * `db_path` - Path to the SQLite database file
pub fn execute(content: String, date: Option<String>, db_path: &Path) -> Result<(), ThoughtError> {
    let date = date.as_deref().map(date_parser::parse_date).transpose()?;

    let mut conn = get_connection(db_path)?;
    run_migrations(&conn)?;

    let (thought_id, entity_names) = thought_writer::create_thought(&mut conn, &content, date)?;

    // Success message with entity count
    if entity_names.is_empty() {
        println!("Thought added successfully (ID: {})", thought_id);
    } else {
        println!(
            "Thought added successfully (ID: {}, {} entity reference{})",
            thought_id,
            entity_names.len(),
            if entity_names.len() == 1 { "" } else { "s" }
        );
    }

    Ok(())
}
