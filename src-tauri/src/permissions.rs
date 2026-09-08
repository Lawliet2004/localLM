use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AccessMode {
    #[default]
    Ask,
    AutoApprove,
    FullAccess,
}
impl AccessMode {
    pub fn automatic_reason(self, trusted_read: bool) -> Option<&'static str> {
        match self {
            Self::Ask => None,
            Self::AutoApprove if trusted_read => Some("auto-approved workspace read"),
            Self::AutoApprove => None,
            Self::FullAccess => Some("full access selected by user"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes_only_grant_their_declared_scope() {
        assert_eq!(AccessMode::default(), AccessMode::Ask);
        for trusted_read in [true, false] {
            assert!(AccessMode::Ask.automatic_reason(trusted_read).is_none());
            assert!(AccessMode::FullAccess
                .automatic_reason(trusted_read)
                .is_some());
        }
        assert!(AccessMode::AutoApprove.automatic_reason(true).is_some());
        assert!(AccessMode::AutoApprove.automatic_reason(false).is_none());
        assert!(serde_json::from_str::<AccessMode>("\"unknown\"").is_err());
    }
    #[test]
    fn old_conversations_default_to_asking_and_invalid_modes_are_rejected() {
        let settings: crate::store::ConversationTools =
            serde_json::from_str(r#"{"sources":[],"tools":[]}"#).unwrap();
        assert_eq!(settings.access_mode, AccessMode::Ask);
        assert!(serde_json::from_str::<crate::store::ConversationTools>(
            r#"{"sources":[],"tools":[],"accessMode":"skipSecurity"}"#
        )
        .is_err());
    }
}
