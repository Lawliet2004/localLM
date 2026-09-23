use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AccessMode {
    #[default]
    Ask,
    AutoApprove,
    FullAccess,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PermissionClass {
    PublicWebRead,
    WorkspaceRead,
    WorkspaceWrite,
    LocalCodeExecution,
    ShellExecution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant {
    Deny,
    Ask,
    AllowForTask,
}

/// Migration does not widen access. Ask stays ask. Auto-approve reads grants
/// workspace read for the task only. Full access keeps the previous breadth
/// and still cannot be granted by model output.
pub fn grants_for(mode: AccessMode) -> [(PermissionClass, Grant); 5] {
    let read = match mode {
        AccessMode::Ask => Grant::Ask,
        AccessMode::AutoApprove => Grant::AllowForTask,
        AccessMode::FullAccess => Grant::AllowForTask,
    };
    let rest = match mode {
        AccessMode::FullAccess => Grant::AllowForTask,
        _ => Grant::Ask,
    };
    [
        (PermissionClass::PublicWebRead, rest),
        (PermissionClass::WorkspaceRead, read),
        (PermissionClass::WorkspaceWrite, rest),
        (PermissionClass::LocalCodeExecution, rest),
        (PermissionClass::ShellExecution, rest),
    ]
}

/// The backend decides. A model-written allow never grants a permission.
pub fn backend_allows(grant: Grant, model_claimed_allow: bool) -> bool {
    if model_claimed_allow {
        return false;
    }
    matches!(grant, Grant::AllowForTask)
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

    #[test]
    fn migration_does_not_broaden_grants_and_the_model_cannot_grant() {
        let ask = super::grants_for(AccessMode::Ask);
        assert!(ask.iter().all(|(_, grant)| *grant == super::Grant::Ask));
        let auto = super::grants_for(AccessMode::AutoApprove);
        assert_eq!(auto[1], (super::PermissionClass::WorkspaceRead, super::Grant::AllowForTask));
        assert!(auto.iter().filter(|(class, _)| *class != super::PermissionClass::WorkspaceRead).all(|(_, grant)| *grant == super::Grant::Ask));
        assert!(!super::backend_allows(super::Grant::Ask, true));
        assert!(!super::backend_allows(super::Grant::Deny, true));
        assert!(!super::backend_allows(super::Grant::AllowForTask, true));
        assert!(super::backend_allows(super::Grant::AllowForTask, false));
        assert!(!super::backend_allows(super::Grant::Deny, false));
    }
}
