use super::{ExecutionRequestV1, ExecutionTarget, EXECUTION_CONTRACT_V1};
use crate::{ExecutionRequest, SandboxError};

impl ExecutionRequestV1 {
    /// Validate the frozen v1 marker, supported fields, and legacy request invariants.
    pub fn validate(&self) -> Result<(), SandboxError> {
        self.validate_supported_fields()?;
        self.as_execution_request().validate()
    }

    /// Convert to the legacy runtime request after confirming no v1 field is silently dropped.
    pub fn into_execution_request(self) -> Result<ExecutionRequest, SandboxError> {
        self.validate_supported_fields()?;
        let request = ExecutionRequest {
            id: self.id,
            language: self.runtime,
            code: self.code,
            limits: self.resources,
            network: self.network,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate_supported_fields(&self) -> Result<(), SandboxError> {
        if self.schema_version != EXECUTION_CONTRACT_V1 {
            return Err(SandboxError::InvalidRequest(
                "schema_version must be v1".into(),
            ));
        }
        if self.target == ExecutionTarget::Remote {
            return Err(SandboxError::UnsupportedFeature(
                "remote execution target".into(),
            ));
        }
        if self.runtime_version.is_some() {
            return Err(SandboxError::UnsupportedFeature(
                "runtime version pinning".into(),
            ));
        }
        if self.entrypoint.is_some() {
            return Err(SandboxError::UnsupportedFeature(
                "custom runtime entrypoint".into(),
            ));
        }
        if self.workspace.is_some() {
            return Err(SandboxError::UnsupportedFeature("workspace binding".into()));
        }
        if !self.artifacts.is_empty() {
            return Err(SandboxError::UnsupportedFeature(
                "artifact inputs or outputs".into(),
            ));
        }
        Ok(())
    }

    fn as_execution_request(&self) -> ExecutionRequest {
        ExecutionRequest {
            id: self.id,
            language: self.runtime,
            code: self.code.clone(),
            limits: self.resources.clone(),
            network: self.network.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        contract::{ArtifactRequest, ErrorCode, ExecutionMode, WorkspaceRef},
        Language, NetworkPolicy, ResourceLimits,
    };
    use uuid::Uuid;

    fn request() -> ExecutionRequestV1 {
        ExecutionRequestV1 {
            schema_version: EXECUTION_CONTRACT_V1.into(),
            id: Uuid::new_v4(),
            runtime: Language::Python,
            runtime_version: None,
            mode: ExecutionMode::Script,
            entrypoint: None,
            workspace: None,
            code: "print('ok')".into(),
            resources: ResourceLimits::default(),
            network: NetworkPolicy::Deny,
            artifacts: Vec::new(),
            target: ExecutionTarget::Local,
        }
    }

    #[test]
    fn supported_v1_fields_map_to_the_runtime_request() {
        let request = request();
        let expected_id = request.id;
        let converted = request.into_execution_request().unwrap();
        assert_eq!(converted.id, expected_id);
        assert_eq!(converted.language, Language::Python);
        assert_eq!(converted.code, "print('ok')");
        assert_eq!(converted.network, NetworkPolicy::Deny);
    }

    #[test]
    fn unsupported_v1_fields_are_rejected_instead_of_silently_dropped() {
        let mut versioned_runtime = request();
        versioned_runtime.runtime_version = Some("python3.12".into());

        let mut entrypoint = request();
        entrypoint.entrypoint = Some("main.py".into());

        let mut workspace = request();
        workspace.workspace = Some(WorkspaceRef {
            id: "workspace-1".into(),
        });

        let mut artifacts = request();
        artifacts.artifacts.push(ArtifactRequest {
            path: "report.html".into(),
            media_type: "text/html".into(),
        });

        let cases = [
            (versioned_runtime, "runtime version pinning"),
            (entrypoint, "custom runtime entrypoint"),
            (workspace, "workspace binding"),
            (artifacts, "artifact inputs or outputs"),
        ];
        for (request, expected) in cases {
            assert!(matches!(
                request.into_execution_request(),
                Err(SandboxError::UnsupportedFeature(feature)) if feature == expected
            ));
        }
    }

    #[test]
    fn remote_target_has_an_explicit_unsupported_error() {
        let mut request = request();
        request.target = ExecutionTarget::Remote;
        assert!(matches!(
            request.validate(),
            Err(SandboxError::UnsupportedFeature(feature)) if feature == "remote execution target"
        ));
    }

    #[test]
    fn unsupported_features_use_the_stable_unsupported_error_code() {
        assert_eq!(
            SandboxError::UnsupportedFeature("workspace binding".into()).code(),
            ErrorCode::Unsupported
        );
    }
}
