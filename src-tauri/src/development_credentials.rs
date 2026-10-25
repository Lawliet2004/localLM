//! Native-only credential creation; model tools receive opaque references.
use serde::{Deserialize, Serialize};
use serde_json::Value;
const KEY: &str = "development.http_credentials";
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpCredential {
    pub id: String,
    pub name: String,
    pub header: String,
    #[serde(default)]
    pub origin: String,
}
#[tauri::command]
pub fn list_http_credentials(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<HttpCredential>, String> {
    state.database()?.setting(KEY)
}
#[tauri::command]
pub async fn save_http_credential(
    state: tauri::State<'_, crate::AppState>,
    name: String,
    header: String,
    secret: String,
    origin: String,
) -> Result<HttpCredential, String> {
    if name.trim().is_empty() || name.len() > 128 || secret.is_empty() || secret.len() > 8192 {
        return Err("Provide a name and a bounded credential value.".into());
    }
    if !["authorization", "x-api-key", "api-key", "cookie"]
        .contains(&header.to_ascii_lowercase().as_str())
    {
        return Err("Choose Authorization, X-API-Key, Api-Key or Cookie.".into());
    }
    reqwest::header::HeaderValue::from_str(&secret)
        .map_err(|_| "Credential contains invalid header characters.".to_string())?;
    let mut items: Vec<HttpCredential> = state.database()?.setting(KEY)?;
    if items.len() >= 32 {
        return Err("At most 32 HTTP credentials may be configured.".into());
    }
    let target =
        reqwest::Url::parse(&origin).map_err(|_| "Provide an exact HTTP(S) credential origin.")?;
    if !matches!(target.scheme(), "http" | "https")
        || target.origin().ascii_serialization() != origin
    {
        return Err("Use an exact credential origin without paths, queries or credentials.".into());
    }
    let credential = HttpCredential {
        id: format!("dev-http-{}", uuid::Uuid::new_v4().simple()),
        name,
        origin,
        header: header.to_ascii_lowercase(),
    };
    state
        .daytona_vault
        .save_blocking(&credential.id, secret.into_bytes())
        .await?;
    items.push(credential.clone());
    let saved = state.database()?.save_setting(KEY, &items);
    if let Err(e) = saved {
        let _ = state.daytona_vault.clear_blocking(&credential.id).await;
        return Err(e);
    }
    Ok(credential)
}
#[tauri::command]
pub async fn delete_http_credential(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<(), String> {
    let mut items: Vec<HttpCredential> = state.database()?.setting(KEY)?;
    if !items.iter().any(|v| v.id == id) {
        return Err("Unknown HTTP credential.".into());
    }
    items.retain(|v| v.id != id);
    state.database()?.save_setting(KEY, &items)?;
    state.daytona_vault.clear_blocking(&id).await
}
pub async fn resolve(
    state: &crate::AppState,
    id: &str,
    target: &reqwest::Url,
) -> Result<(String, String), String> {
    let items: Vec<HttpCredential> = state.database()?.setting(KEY)?;
    let item = items
        .iter()
        .find(|v| v.id == id)
        .ok_or("HTTP credential is not configured.")?;
    if item.origin != target.origin().ascii_serialization() {
        return Err(
            "HTTP credential origin does not match this request. Update its native configuration."
                .into(),
        );
    }
    let bytes = state
        .daytona_vault
        .load_blocking(&item.id)
        .await?
        .ok_or("HTTP credential is unavailable in the vault.")?;
    Ok((
        item.header.clone(),
        String::from_utf8(bytes).map_err(|_| "Invalid stored credential.".to_string())?,
    ))
}
#[tauri::command]
pub fn get_reference_origins(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<String>, String> {
    state.database()?.setting("development.reference_origins")
}
#[tauri::command]
pub fn save_reference_origins(
    state: tauri::State<'_, crate::AppState>,
    origins: Vec<String>,
) -> Result<(), String> {
    if origins.len() > 16 {
        return Err("Configure at most 16 OpenAPI reference origins.".into());
    }
    for origin in &origins {
        let url =
            reqwest::Url::parse(origin).map_err(|_| "Invalid reference origin.".to_string())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.origin().ascii_serialization() != *origin
        {
            return Err("Use exact HTTP(S) origins without paths, queries or credentials.".into());
        }
    }
    state
        .database()?
        .save_setting("development.reference_origins", &origins)
}
pub fn structural_diff(a: &Value, b: &Value) -> Value {
    fn walk(a: &Value, b: &Value, path: &str, out: &mut Vec<Value>, omitted: &mut bool) {
        if a == b {
            return;
        }
        if out.len() >= 100 {
            *omitted = true;
            return;
        }
        match(a,b){(Value::Object(a),Value::Object(b))=>{let keys=a.keys().chain(b.keys()).collect::<std::collections::BTreeSet<_>>();for k in keys{let p=format!("{}/{}",path,k.replace('~',"~0").replace('/',"~1"));match(a.get(k),b.get(k)){(Some(a),Some(b))=>walk(a,b,&p,out,omitted),(None,Some(_))=>out.push(serde_json::json!({"path":p,"change":"added"})),(Some(_),None)=>out.push(serde_json::json!({"path":p,"change":"removed"})),_=>()};if out.len()>=100{*omitted=true;break;}}},(Value::Array(a),Value::Array(b))=>for i in 0..a.len().max(b.len()).min(101){let p=format!("{path}/{i}");match(a.get(i),b.get(i)){(Some(a),Some(b))=>walk(a,b,&p,out,omitted),(None,Some(_))=>out.push(serde_json::json!({"path":p,"change":"added"})),(Some(_),None)=>out.push(serde_json::json!({"path":p,"change":"removed"})),_=>()};if out.len()>=100{*omitted=true;break;}},_=>out.push(serde_json::json!({"path":path,"change":"value_or_type_changed","beforeType":kind(a),"afterType":kind(b),"beforeSha256":crate::development_tools::hash(a.to_string().as_bytes()),"afterSha256":crate::development_tools::hash(b.to_string().as_bytes())}))}
    }
    fn kind(v: &Value) -> &str {
        match v {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
    let mut changes = Vec::new();
    let mut truncated = false;
    walk(a, b, "", &mut changes, &mut truncated);
    serde_json::json!({"changes":changes,"truncated":truncated})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn opaque_credentials_roundtrip_in_the_vault_and_are_origin_bound() {
        let t = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(t.path());
        let item = HttpCredential {
            id: format!("dev-http-{}", uuid::Uuid::new_v4().simple()),
            name: "Fixture".into(),
            header: "authorization".into(),
            origin: "https://api.example.com".into(),
        };
        state
            .daytona_vault
            .save(&item.id, b"Bearer fixture-secret")
            .unwrap();
        state
            .database()
            .unwrap()
            .save_setting(KEY, &vec![item.clone()])
            .unwrap();
        assert_eq!(
            resolve(
                &state,
                &item.id,
                &reqwest::Url::parse("https://api.example.com/path").unwrap()
            )
            .await
            .unwrap()
            .1,
            "Bearer fixture-secret"
        );
        assert!(resolve(
            &state,
            &item.id,
            &reqwest::Url::parse("https://other.example.com/").unwrap()
        )
        .await
        .is_err());
        assert!(!serde_json::to_string(&item)
            .unwrap()
            .contains("fixture-secret"));
    }
    #[test]
    fn structural_diffs_escape_json_pointer_paths_and_bound_changes() {
        let diff = structural_diff(&json!({"a/b":{"~key":1}}), &json!({"a/b":{"~key":"1"}}));
        assert_eq!(diff["changes"][0]["path"], "/a~1b/~0key");
        assert_eq!(diff["changes"][0]["beforeType"], "number");
        let a = json!([]);
        let b = json!((0..500).collect::<Vec<_>>());
        let result = structural_diff(&a, &b);
        assert!(result["changes"].as_array().unwrap().len() <= 100);
        assert_eq!(result["truncated"], true);
    }
}
