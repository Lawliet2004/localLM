use rmcp::model::{ListToolsResult, Tool};
use std::{collections::HashSet, future::Future};

/// Bound accumulated catalog data and pagination independently of the outer request deadline.
pub async fn discover<F, Fut>(mut page: F) -> Result<Vec<Tool>, String>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<ListToolsResult, String>>,
{
    let mut cursor = None;
    let mut cursors = HashSet::new();
    let mut names = HashSet::new();
    let mut tools = Vec::new();
    let mut bytes = 0_usize;
    for _ in 0..64 {
        let result = page(cursor).await?;
        if tools.len() + result.tools.len() > 512 {
            return Err("Connector returned more than 512 tools.".into());
        }
        for tool in result.tools {
            if !names.insert(tool.name.to_string()) {
                return Err("Connector returned duplicate tool names.".into());
            }
            bytes += serde_json::to_vec(&tool)
                .map_err(|error| error.to_string())?
                .len();
            if bytes > 2_097_152 {
                return Err("Connector tool catalog exceeds 2 MiB.".into());
            }
            tools.push(tool);
        }
        let Some(next) = result.next_cursor else {
            return Ok(tools);
        };
        if next.len() > 4096 || !cursors.insert(next.clone()) {
            return Err("Connector returned an invalid or repeated pagination cursor.".into());
        }
        cursor = Some(next);
    }
    Err("Connector tool discovery exceeded 64 pages.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn rejects_repeated_cursors_duplicate_names_and_excessive_pages() {
        for case in ["valid", "cursor", "duplicate", "pages", "count"] {
            let mut calls = 0;
            let result = discover(|_| {
                calls += 1;
                let next = if case == "valid" && calls == 2 { None } else if case == "cursor" { Some("repeated".to_string()) } else { Some(calls.to_string()) };
                let tools = if case == "count" { (0..513).map(|index| json!({"name":format!("t{index}"),"inputSchema":{"type":"object"}})).collect::<Vec<_>>() }
                    else if case == "pages" { vec![] }
                    else { vec![json!({"name":if case == "duplicate" { "same".into() } else { format!("t{calls}") },"inputSchema":{"type":"object"}})] };
                let value = serde_json::from_value(json!({"tools":tools,"nextCursor":next})).unwrap();
                async move { Ok(value) }
            }).await;
            assert_eq!(result.is_ok(), case == "valid");
            assert!(calls <= 64);
            if case == "cursor" || case == "duplicate" {
                assert_eq!(calls, 2);
            }
            if case == "valid" {
                assert_eq!(result.unwrap().len(), 2);
            }
        }
    }
}
