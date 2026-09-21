use crate::error::{StudioError, StudioResult};

use super::{
    Architecture, AvailableRelease, ComponentPolicy, OperatingSystem, Platform, ReleaseAsset,
    ReleaseAssetKind,
};

/// Isolated host-platform detector. Production detection uses Rust's compile-time
/// target identity; tests use `from_raw` so normalization/failure behavior is
/// deterministic and does not depend on the machine running the test suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPlatform {
    platform: Platform,
}

impl HostPlatform {
    pub fn detect() -> StudioResult<Self> {
        Self::from_raw(std::env::consts::OS, std::env::consts::ARCH)
    }

    pub fn from_raw(os: &str, arch: &str) -> StudioResult<Self> {
        Ok(Self {
            platform: Platform {
                os: normalize_os(os)?,
                arch: normalize_arch(arch)?,
            },
        })
    }

    pub const fn platform(self) -> Platform {
        self.platform
    }
}

pub fn normalize_os(value: &str) -> StudioResult<OperatingSystem> {
    match value.to_ascii_lowercase().as_str() {
        // `std::env::consts::OS` reports `macos`; GitHub release assets use
        // the Darwin platform identity.
        "darwin" | "macos" => Ok(OperatingSystem::Darwin),
        _ => Err(StudioError::UnsupportedOperatingSystem(value.to_owned())),
    }
}

pub fn normalize_arch(value: &str) -> StudioResult<Architecture> {
    match value.to_ascii_lowercase().as_str() {
        "arm64" | "aarch64" => Ok(Architecture::Arm64),
        _ => Err(StudioError::UnsupportedArchitecture(value.to_owned())),
    }
}

/// Resolve one exact archive name from trusted component packaging metadata.
/// This function is deliberately transport-free: providers supply already
/// validated release metadata and M5.5 later owns download/verification.
pub fn select_release_asset<'a>(
    component: &ComponentPolicy,
    release: &'a AvailableRelease,
    platform: Platform,
) -> StudioResult<&'a ReleaseAsset> {
    if release.component != component.id {
        return Err(StudioError::ReleaseComponentMismatch {
            expected: component.id.to_string(),
            actual: release.component.to_string(),
        });
    }

    let expected = expected_asset_name(component, release, platform);
    let matches = release
        .assets
        .iter()
        .filter(|asset| asset.name == expected)
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [asset] => Ok(*asset),
        [] => Err(StudioError::ReleaseAssetNotFound {
            component: component.id.to_string(),
            expected,
        }),
        _ => Err(StudioError::AmbiguousReleaseAsset {
            component: component.id.to_string(),
            expected,
            count: matches.len(),
        }),
    }
}

pub fn expected_asset_name(
    component: &ComponentPolicy,
    release: &AvailableRelease,
    platform: Platform,
) -> String {
    match component.asset.kind {
        ReleaseAssetKind::PlatformTarGz => {
            format!(
                "{}-v{}-{platform}.tar.gz",
                component.asset.stem, release.version
            )
        }
        ReleaseAssetKind::ArchitectureIndependentTarGz => {
            format!("{}-v{}.tar.gz", component.asset.stem, release.version)
        }
        ReleaseAssetKind::PlatformZip => {
            format!(
                "{}-v{}-{platform}.zip",
                component.asset.stem, release.version
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::update::{ComponentCatalog, ComponentId, HostRuntimeRoots, ReleaseAsset, Version};

    fn catalog() -> ComponentCatalog {
        ComponentCatalog::new(
            HostRuntimeRoots::new(
                PathBuf::from("/trusted/bin"),
                PathBuf::from("/trusted/runtime"),
            )
            .unwrap(),
        )
    }

    fn platform(arch: Architecture) -> Platform {
        Platform {
            os: OperatingSystem::Darwin,
            arch,
        }
    }

    fn release(component: ComponentId, version: &str, names: &[&str]) -> AvailableRelease {
        let version = Version::parse(version).unwrap();
        AvailableRelease {
            component,
            tag: format!("v{version}"),
            version,
            assets: names
                .iter()
                .map(|name| ReleaseAsset {
                    name: (*name).to_owned(),
                    download_url: format!("https://example.invalid/{name}"),
                })
                .collect(),
            checksum_manifest_url: "https://example.invalid/SHA256SUMS.txt".into(),
        }
    }

    #[test]
    fn normalizes_darwin_arm64_to_darwin_arm64() {
        assert_eq!(
            HostPlatform::from_raw("Darwin", "arm64")
                .unwrap()
                .platform()
                .to_string(),
            "darwin-arm64"
        );
    }

    #[test]
    fn normalization_accepts_required_aliases_and_rust_macos_identity() {
        assert_eq!(normalize_os("darwin").unwrap(), OperatingSystem::Darwin);
        assert_eq!(normalize_os("macos").unwrap(), OperatingSystem::Darwin);
        assert_eq!(normalize_arch("arm64").unwrap(), Architecture::Arm64);
        assert_eq!(normalize_arch("aarch64").unwrap(), Architecture::Arm64);
    }

    #[test]
    fn unsupported_os_and_architecture_fail_closed() {
        assert!(matches!(
            HostPlatform::from_raw("linux", "arm64").unwrap_err(),
            StudioError::UnsupportedOperatingSystem(value) if value == "linux"
        ));
        assert!(matches!(
            HostPlatform::from_raw("Darwin", "riscv64").unwrap_err(),
            StudioError::UnsupportedArchitecture(value) if value == "riscv64"
        ));
        assert!(matches!(
            HostPlatform::from_raw("Darwin", "x86_64").unwrap_err(),
            StudioError::UnsupportedArchitecture(value) if value == "x86_64"
        ));
    }

    #[test]
    fn selects_exact_project_rust_asset_and_rejects_wrong_arch_or_version() {
        let catalog = catalog();
        let component = catalog.component(ComponentId::Git).unwrap();
        let release = release(
            ComponentId::Git,
            "1.2.3",
            &[
                "rust-mcp-git-v1.2.2-darwin-arm64.tar.gz",
                "rust-mcp-git-v1.2.3-darwin-amd64.tar.gz",
                "rust-mcp-git-v1.2.3-darwin-arm64.tar.gz",
                "rust-mcp-git-v1.2.3-darwin-arm64.tar.gz.spdx.json",
            ],
        );

        assert_eq!(
            select_release_asset(component, &release, platform(Architecture::Arm64))
                .unwrap()
                .name,
            "rust-mcp-git-v1.2.3-darwin-arm64.tar.gz"
        );
    }

    #[test]
    fn selects_exact_studio_asset() {
        let catalog = catalog();
        let component = catalog.component(ComponentId::Studio).unwrap();
        let release = release(
            ComponentId::Studio,
            "0.5.0",
            &["mcp-studio-v0.5.0-darwin-arm64.tar.gz"],
        );
        assert_eq!(
            select_release_asset(component, &release, platform(Architecture::Arm64))
                .unwrap()
                .name,
            "mcp-studio-v0.5.0-darwin-arm64.tar.gz"
        );
    }

    #[test]
    fn fleet_is_explicitly_architecture_independent() {
        let catalog = catalog();
        let component = catalog.component(ComponentId::Fleet).unwrap();
        assert_eq!(
            component.asset.kind,
            ReleaseAssetKind::ArchitectureIndependentTarGz
        );
        let release = release(ComponentId::Fleet, "0.2.0", &["mcp-fleet-v0.2.0.tar.gz"]);

        assert_eq!(
            select_release_asset(component, &release, platform(Architecture::Arm64))
                .unwrap()
                .name,
            "mcp-fleet-v0.2.0.tar.gz"
        );
    }

    #[test]
    fn selects_exact_official_tunnel_runtime_zip() {
        let catalog = catalog();
        let component = catalog.component(ComponentId::Tunnel).unwrap();
        let release = release(
            ComponentId::Tunnel,
            "0.0.14",
            &[
                "tunnel-client-runtime-cloudflared-v0.0.14-darwin-amd64.zip",
                "tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64-licenses.txt",
                "tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64.spdx.json",
                "tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64.zip",
                "tunnel-client-v0.0.14-darwin-arm64.zip",
            ],
        );
        assert_eq!(
            select_release_asset(component, &release, platform(Architecture::Arm64))
                .unwrap()
                .name,
            "tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64.zip"
        );
    }

    #[test]
    fn every_catalog_component_has_an_explicit_asset_contract() {
        let catalog = catalog();
        let darwin_arm64 = platform(Architecture::Arm64);
        let cases = [
            (
                ComponentId::Filesystem,
                "1.0.0",
                "rust-mcp-filesystem-v1.0.0-darwin-arm64.tar.gz",
            ),
            (
                ComponentId::Git,
                "1.0.0",
                "rust-mcp-git-v1.0.0-darwin-arm64.tar.gz",
            ),
            (
                ComponentId::Exec,
                "1.0.0",
                "rust-mcp-exec-v1.0.0-darwin-arm64.tar.gz",
            ),
            (
                ComponentId::Gateway,
                "1.0.0",
                "rust-mcp-gateway-v1.0.0-darwin-arm64.tar.gz",
            ),
            (
                ComponentId::Blender,
                "1.0.0",
                "rust-mcp-blender-v1.0.0-darwin-arm64.tar.gz",
            ),
            (
                ComponentId::Studio,
                "0.5.0",
                "mcp-studio-v0.5.0-darwin-arm64.tar.gz",
            ),
            (ComponentId::Fleet, "0.2.0", "mcp-fleet-v0.2.0.tar.gz"),
            (
                ComponentId::Tunnel,
                "0.0.14",
                "tunnel-client-runtime-cloudflared-v0.0.14-darwin-arm64.zip",
            ),
        ];

        for (id, version, expected) in cases {
            let component = catalog.component(id).unwrap();
            let release = release(id, version, &[expected]);
            assert_eq!(
                expected_asset_name(component, &release, darwin_arm64),
                expected,
                "unexpected asset contract for {id}"
            );
            assert_eq!(
                select_release_asset(component, &release, darwin_arm64)
                    .unwrap()
                    .name,
                expected
            );
        }
    }

    #[test]
    fn missing_and_duplicate_exact_assets_fail_closed() {
        let catalog = catalog();
        let component = catalog.component(ComponentId::Git).unwrap();
        let missing = release(
            ComponentId::Git,
            "1.0.0",
            &["rust-mcp-git-v1.0.0-darwin-amd64.tar.gz"],
        );
        assert!(matches!(
            select_release_asset(component, &missing, platform(Architecture::Arm64)).unwrap_err(),
            StudioError::ReleaseAssetNotFound { component, expected }
                if component == "git" && expected == "rust-mcp-git-v1.0.0-darwin-arm64.tar.gz"
        ));

        let duplicate = release(
            ComponentId::Git,
            "1.0.0",
            &[
                "rust-mcp-git-v1.0.0-darwin-arm64.tar.gz",
                "rust-mcp-git-v1.0.0-darwin-arm64.tar.gz",
            ],
        );
        assert!(matches!(
            select_release_asset(component, &duplicate, platform(Architecture::Arm64)).unwrap_err(),
            StudioError::AmbiguousReleaseAsset { component, expected, count: 2 }
                if component == "git" && expected == "rust-mcp-git-v1.0.0-darwin-arm64.tar.gz"
        ));
    }

    #[test]
    fn release_component_mismatch_is_rejected() {
        let catalog = catalog();
        let git = catalog.component(ComponentId::Git).unwrap();
        let release = release(
            ComponentId::Exec,
            "1.0.0",
            &["rust-mcp-git-v1.0.0-darwin-arm64.tar.gz"],
        );
        assert!(matches!(
            select_release_asset(git, &release, platform(Architecture::Arm64)).unwrap_err(),
            StudioError::ReleaseComponentMismatch { expected, actual }
                if expected == "git" && actual == "exec"
        ));
    }
}
