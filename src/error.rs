use thiserror::Error;

#[derive(Debug, Error)]
pub enum StudioError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("MCP server not found: {0}")]
    NotFound(String),

    #[error("managed component not found: {0}")]
    UnknownComponent(String),

    #[error("installed artifact identity error: {0}")]
    InstalledIdentity(String),

    #[error("release provider unreachable for {component}: {detail}")]
    ReleaseProviderUnreachable { component: String, detail: String },

    #[error("release provider HTTP error for {component}: status {status}")]
    ReleaseProviderHttp { component: String, status: u16 },

    #[error("release provider response exceeded {limit} bytes for {component}")]
    ReleaseProviderResponseTooLarge { component: String, limit: usize },

    #[error("malformed release metadata for {component}: {detail}")]
    MalformedReleaseMetadata { component: String, detail: String },

    #[error("invalid release tag for {component}: {tag:?}: {detail}")]
    InvalidReleaseTag {
        component: String,
        tag: String,
        detail: String,
    },

    #[error("no stable release is available for {component}")]
    NoStableRelease { component: String },

    #[error("release {version} was not found for {component}")]
    ReleaseVersionNotFound { component: String, version: String },

    #[error("release {tag} is not eligible for the stable channel of {component}: {reason}")]
    ReleaseNotEligible {
        component: String,
        tag: String,
        reason: String,
    },

    #[error("release metadata for {component} is missing SHA256SUMS.txt")]
    MissingChecksumManifest { component: String },

    #[error("release metadata for {component} contains multiple SHA256SUMS.txt assets")]
    DuplicateChecksumManifest { component: String },

    #[error("release metadata for {component} contains an untrusted asset URL: {url}")]
    UntrustedReleaseAssetUrl { component: String, url: String },

    #[error("release metadata for {component} has no asset in required family {family}")]
    MissingReleaseAssetFamily { component: String, family: String },

    #[error("release metadata for {component} has ambiguous assets in family {family}: {detail}")]
    AmbiguousReleaseAssetFamily {
        component: String,
        family: String,
        detail: String,
    },

    #[error("unsupported runtime operating system: {0}")]
    UnsupportedOperatingSystem(String),

    #[error("unsupported runtime architecture: {0}")]
    UnsupportedArchitecture(String),

    #[error("release metadata component mismatch: expected {expected}, got {actual}")]
    ReleaseComponentMismatch { expected: String, actual: String },

    #[error("release metadata for {component} is missing expected asset {expected}")]
    ReleaseAssetNotFound { component: String, expected: String },

    #[error(
        "release metadata for {component} contains {count} copies of expected asset {expected}"
    )]
    AmbiguousReleaseAsset {
        component: String,
        expected: String,
        count: usize,
    },

    #[error("release provider mismatch for {component}: expected {expected}, got {actual}")]
    ReleaseProviderMismatch {
        component: String,
        expected: String,
        actual: String,
    },

    #[error("artifact download failed for {component}: {detail}")]
    ArtifactDownloadFailed { component: String, detail: String },

    #[error("artifact exceeds staging size limit of {limit} bytes")]
    ArtifactTooLarge { limit: u64 },

    #[error("invalid checksum manifest for {component}: {detail}")]
    InvalidChecksumManifest { component: String, detail: String },

    #[error("checksum manifest for {component} has no entry for {asset}")]
    ChecksumEntryMissing { component: String, asset: String },

    #[error("checksum manifest for {component} has multiple entries for {asset}")]
    ChecksumEntryAmbiguous { component: String, asset: String },

    #[error("checksum mismatch for {component}/{asset}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        component: String,
        asset: String,
        expected: String,
        actual: String,
    },

    #[error("unsafe archive for {component}: {detail}")]
    UnsafeArchive { component: String, detail: String },

    #[error("package validation failed for {component}: {detail}")]
    PackageValidationFailed { component: String, detail: String },

    #[error("binary version validation failed for {path}: {detail}")]
    BinaryVersionValidationFailed { path: String, detail: String },

    #[error("staging failure: {0}")]
    StagingFailure(String),

    #[error("update transaction failure: {0}")]
    UpdateTransaction(String),

    #[error("update activation failed for {component}: {detail}")]
    UpdateActivationFailed { component: String, detail: String },

    #[error("update verification failed for {component}: {detail}")]
    UpdateVerificationFailed { component: String, detail: String },

    #[error("rollback failed for {component}: {detail}")]
    RollbackFailed { component: String, detail: String },

    #[error("duplicate MCP registration: {0}")]
    Duplicate(String),

    #[error("MCP server is disabled: {0}")]
    Disabled(String),

    #[error("MCP registry mutation conflicts with runtime state: {0}")]
    Conflict(String),

    #[error("unsupported MCP project: {0}")]
    Unsupported(String),

    #[error("MCP server is already running: {0}")]
    AlreadyRunning(String),

    #[error("MCP server is not running: {0}")]
    NotRunning(String),

    #[error("process error: {0}")]
    Process(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML decode error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("TOML encode error: {0}")]
    TomlEncode(#[from] toml::ser::Error),

    #[error("JSON decode error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type StudioResult<T> = Result<T, StudioError>;
