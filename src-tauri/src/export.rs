use crate::store::{Conversation, ConversationTools, Message, Store};
use serde::Serialize;
use std::{io::Write, path::Path};

const MAX_EXPORT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    schema_version: u32,
    exported_at: i64,
    conversation: Conversation,
    tool_selection: ConversationTools,
    messages: Vec<Message>,
}

fn snapshot(store: &Store, id: &str) -> Result<Snapshot, String> {
    let conversation = store
        .list_conversations()?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or("Conversation no longer exists.")?;
    Ok(Snapshot {
        schema_version: 1,
        exported_at: crate::store::now(),
        conversation,
        tool_selection: store.conversation_tools(id)?,
        messages: store.messages(id)?,
    })
}

struct BoundedOutput(Vec<u8>);
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_EXPORT_BYTES {
            return Err(std::io::Error::other("Conversation export exceeds 64 MiB."));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn render(snapshot: &Snapshot, extension: &str) -> Result<Vec<u8>, String> {
    let mut output = BoundedOutput(Vec::new());
    if extension == "json" {
        serde_json::to_writer_pretty(&mut output, snapshot).map_err(|error| error.to_string())?;
    } else if extension == "md" || extension == "markdown" {
        let title = snapshot.conversation.title.replace(['\r', '\n'], " ");
        writeln!(
            output,
            "# {title}\n\nConversation: {}\n\nExported at: {} (Unix milliseconds)\n",
            snapshot.conversation.id, snapshot.exported_at
        )
        .map_err(|error| error.to_string())?;
        for message in &snapshot.messages {
            writeln!(
                output,
                "## {}\n\nStatus: {} · Created at: {} (Unix milliseconds)\n",
                message.role, message.status, message.created_at
            )
            .map_err(|error| error.to_string())?;
            if let Some(error) = &message.error {
                writeln!(output, "Response error: {error}\n").map_err(|error| error.to_string())?;
            }
            if !message.reasoning.is_empty() {
                writeln!(
                    output,
                    "### Reasoning\n\n{}\n\n### Response\n",
                    message.reasoning
                )
                .map_err(|error| error.to_string())?;
            }
            if message.role == "tool" {
                // A tool result may itself contain Markdown fences. Keep its full audit literal.
                let longest = message
                    .content
                    .split(|character| character != '`')
                    .map(str::len)
                    .max()
                    .unwrap_or(0);
                let fence = "`".repeat(longest.saturating_add(1).max(3));
                writeln!(output, "{fence}json\n{}\n{fence}\n", message.content)
                    .map_err(|error| error.to_string())?;
            } else {
                writeln!(output, "{}\n", message.content).map_err(|error| error.to_string())?;
            }
        }
    } else {
        return Err("Choose a .md, .markdown or .json file for the export.".into());
    }
    Ok(output.0)
}

fn write_snapshot(snapshot: &Snapshot, path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err("Choose an absolute export file path.".into());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = render(snapshot, &extension)?;
    let parent = path.parent().ok_or("Choose an export folder.")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("Could not create export: {error}"))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("Could not write export: {error}"))?;
    temporary
        .persist(path)
        .map_err(|error| format!("Could not save export: {}", error.error))?;
    Ok(())
}

#[tauri::command]
pub async fn export_conversation(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    path: String,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before exporting.")?;
    let snapshot = snapshot(&*state.database()?, &id)?;
    tokio::task::spawn_blocking(move || write_snapshot(&snapshot, Path::new(&path)))
        .await
        .map_err(|_| "Export worker failed unexpectedly.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Snapshot {
        Snapshot {
            schema_version: 1,
            exported_at: 123,
            conversation: Conversation {
                id: "chat".into(),
                title: "Unicode 日本語".into(),
                updated_at: 100,
                provider_id: None,
                model_id: None,
                provider_selection_required: false,
            },
            tool_selection: ConversationTools::default(),
            messages: vec![
                Message {
                    error: None,
                    id: "assistant".into(),
                    conversation_id: "chat".into(),
                    role: "assistant".into(),
                    content: "Partial answer λ".into(),
                    reasoning: "Thinking evidence".into(),
                    status: "interrupted".into(),
                    created_at: 110,
                },
                Message {
                    error: None,
                    id: "tool".into(),
                    conversation_id: "chat".into(),
                    role: "tool".into(),
                    content: "{\"result\":\"``` hostile fence\"}".into(),
                    reasoning: String::new(),
                    status: "complete".into(),
                    created_at: 120,
                },
            ],
        }
    }
    #[test]
    fn exports_full_json_and_readable_markdown_without_losing_audits() {
        let snapshot = fixture();
        let json: serde_json::Value =
            serde_json::from_slice(&render(&snapshot, "json").unwrap()).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["messages"][0]["reasoning"], "Thinking evidence");
        assert_eq!(json["messages"][0]["status"], "interrupted");
        assert_eq!(json["messages"][1]["content"], snapshot.messages[1].content);
        let markdown = String::from_utf8(render(&snapshot, "md").unwrap()).unwrap();
        assert!(markdown.contains("Unicode 日本語"));
        assert!(markdown.contains("Partial answer λ"));
        assert!(markdown.contains("Status: interrupted"));
        assert!(markdown.contains("````json\n{\"result\":\"``` hostile fence\"}\n````"));
    }
    #[test]
    fn saves_and_replaces_atomically_and_preserves_destination_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.json");
        std::fs::write(&path, "previous export").unwrap();
        write_snapshot(&fixture(), &path).unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(serde_json::from_slice::<serde_json::Value>(&saved).is_ok());
        let protected = dir.path().join("chat.sqlite");
        std::fs::write(&protected, "original").unwrap();
        assert!(write_snapshot(&fixture(), &protected).is_err());
        assert_eq!(std::fs::read_to_string(&protected).unwrap(), "original");
        assert!(write_snapshot(&fixture(), &dir.path().join("missing/chat.json")).is_err());
        assert!(write_snapshot(&fixture(), Path::new("relative.json")).is_err());
        let directory_target = dir.path().join("directory.json");
        std::fs::create_dir(&directory_target).unwrap();
        std::fs::write(directory_target.join("sentinel"), "preserve me").unwrap();
        assert!(write_snapshot(&fixture(), &directory_target).is_err());
        assert_eq!(
            std::fs::read_to_string(directory_target.join("sentinel")).unwrap(),
            "preserve me"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
    }
}
