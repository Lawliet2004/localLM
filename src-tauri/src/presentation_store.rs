//! Additive SQLite presentation/state storage. Model history remains in the incumbent tables.
use crate::presentation::{Mode, ModeSettings, Record, LIBRARY_VERSION, MAX_PROGRAM};
use crate::store::Store;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
pub fn initialize(connection: &rusqlite::Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS message_presentations(
        message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
        record_json TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS conversation_presentation_modes(
        conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
        mode_json TEXT NOT NULL);
        CREATE TRIGGER IF NOT EXISTS presentation_answer_edit AFTER UPDATE OF content ON messages
        WHEN OLD.content!=NEW.content AND OLD.status!='streaming' BEGIN
          DELETE FROM message_presentations WHERE message_id=NEW.id;
        END;",
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// One-shot crash recovery for presentation metadata.
///
/// Recovery changes metadata only, never a completed authoritative answer.
/// It is a **startup** action, not a connection action: running it on a
/// second connection to the live database would mark a presentation that is
/// still being generated as interrupted. Call this exactly once, at
/// startup, on the authoritative connection.
pub fn recover_interrupted(connection: &rusqlite::Connection) -> Result<(), String> {
    connection.execute("UPDATE message_presentations SET record_json=json_set(record_json,'$.generationStatus','interrupted') WHERE CASE WHEN json_valid(record_json) THEN json_extract(record_json,'$.generationStatus')='streaming' ELSE 0 END", []).map_err(|e| e.to_string())?;
    Ok(())
}
impl Store {
    pub fn reject_presentation(
        &self,
        conversation: &str,
        message: &str,
        program: &str,
    ) -> Result<(), String> {
        if program.len() > MAX_PROGRAM {
            return Err("Program too large.".into());
        }
        let changed = self.connection.execute("UPDATE message_presentations SET record_json=json_set(record_json,'$.generationStatus','error','$.diagnostic','UI validation or rendering failed; original answer preserved.') WHERE message_id=?1 AND message_id IN(SELECT id FROM messages WHERE conversation_id=?2 AND role='assistant') AND json_extract(record_json,'$.uiProgram')=?3 AND json_extract(record_json,'$.generationStatus')='complete'", params![message,conversation,program]).map_err(|e|e.to_string())?;
        if changed == 1 {
            self.append_session_event(&crate::sessions::SessionEvent {
                id:uuid::Uuid::new_v4().to_string(),tool_call_id:None,ignorable:false,conversation_id:conversation.into(),seq:0,run_id:None,step_id:Some("presentation".into()),event_type:"presentation_rejected".into(),payload:serde_json::json!({"messageId":message,"renderStatus":"failed","diagnostic":"UI validation or rendering failed; original answer preserved."}),created_at:crate::store::now()
            })?;
        }
        Ok(())
    }

    pub fn presentation_mode(&self, id: Option<&str>) -> Result<ModeSettings, String> {
        let default_mode: Mode = self.setting("presentation.default")?;
        let raw: Option<String> = match id { Some(id) => self.connection.query_row("SELECT mode_json FROM conversation_presentation_modes WHERE conversation_id=?1", [id], |r| r.get(0)).optional().map_err(|e| e.to_string())?, None => None };
        let override_mode = raw
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| e.to_string())?;
        Ok(ModeSettings {
            default_mode,
            override_mode,
            effective_mode: override_mode.unwrap_or(default_mode),
        })
    }
    pub fn set_presentation_mode(
        &self,
        id: Option<&str>,
        mode: Option<Mode>,
    ) -> Result<(), String> {
        if let Some(id) = id {
            if let Some(mode) = mode {
                self.connection.execute("INSERT INTO conversation_presentation_modes VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET mode_json=excluded.mode_json", params![id, serde_json::to_string(&mode).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
            } else {
                self.connection
                    .execute(
                        "DELETE FROM conversation_presentation_modes WHERE conversation_id=?1",
                        [id],
                    )
                    .map_err(|e| e.to_string())?;
            }
        } else {
            self.save_setting("presentation.default", &mode.unwrap_or_default())?;
        }
        Ok(())
    }
    pub fn presentations(&self, id: &str) -> Result<Vec<Record>, String> {
        let mut query = self.connection.prepare("SELECT p.record_json,m.id,m.content FROM message_presentations p JOIN messages m ON m.id=p.message_id WHERE m.conversation_id=?1 ORDER BY m.created_at,m.rowid").map_err(|e| e.to_string())?;
        let rows = query
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            let (raw, message, source) = row.map_err(|e| e.to_string())?;
            Ok(serde_json::from_str(&raw).unwrap_or_else(|_| Record {
                message_id: message,
                format: "markdown".into(),
                format_version: 1,
                library_version: LIBRARY_VERSION.into(),
                source_text: source,
                ui_program: String::new(),
                interactive_state: serde_json::json!({}),
                generation_status: "error".into(),
                diagnostic: Some(
                    "Stored visual metadata was invalid; original answer preserved.".into(),
                ),
            }))
        })
        .collect()
    }
    pub fn save_presentation(&self, record: &Record) -> Result<(), String> {
        if record.ui_program.len() > MAX_PROGRAM || record.source_text.len() > 4_194_304 {
            return Err("Presentation exceeds limits.".into());
        }
        self.connection.execute("INSERT INTO message_presentations VALUES(?1,?2) ON CONFLICT(message_id) DO UPDATE SET record_json=excluded.record_json", params![record.message_id, serde_json::to_string(record).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn save_visual_state(
        &self,
        conversation: &str,
        message: &str,
        state: Value,
    ) -> Result<(), String> {
        fn bounded(value: &Value, depth: usize, count: &mut usize) -> bool {
            *count += 1;
            if depth > 12 || *count > 2048 {
                return false;
            }
            match value {
                Value::Object(map) => map.iter().all(|(key, value)| {
                    !matches!(key.as_str(), "__proto__" | "constructor" | "prototype")
                        && bounded(value, depth + 1, count)
                }),
                Value::Array(a) => a.len() <= 256 && a.iter().all(|v| bounded(v, depth + 1, count)),
                _ => true,
            }
        }
        if !state.is_object() || state.to_string().len() > 32_768 || !bounded(&state, 0, &mut 0) {
            return Err("Invalid visual state.".into());
        }
        let changed = self.connection.execute("UPDATE message_presentations SET record_json=json_set(record_json,'$.interactiveState',json(?3)) WHERE message_id=?2 AND message_id IN (SELECT id FROM messages WHERE conversation_id=?1 AND role='assistant') AND json_extract(record_json,'$.libraryVersion')=?4 AND json_extract(record_json,'$.generationStatus')='complete'", params![conversation,message,state.to_string(),LIBRARY_VERSION]).map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("Visual state does not belong to a completed compatible message in this conversation.".into());
        }
        Ok(())
    }
}
