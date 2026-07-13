#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use pi_rs_app::startup::{
    ImportOutcome, ProbeOutcome, Resource, ResourceSource, StartupContext, StartupEnvironment,
    StartupPathError, StartupPaths,
};

fn environment(home: &Path) -> StartupEnvironment {
    [(OsString::from("HOME"), home.as_os_str().to_owned())]
        .into_iter()
        .collect()
}

fn paths(home: &Path) -> StartupPaths {
    StartupPaths::from_environment(&environment(home)).unwrap()
}

fn create_entry(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

#[derive(Debug, PartialEq, Eq)]
struct Fingerprint {
    bytes: Vec<u8>,
    len: u64,
    modified: std::time::SystemTime,
    readonly: bool,
    #[cfg(unix)]
    mode: u32,
}

fn fingerprint(path: &Path) -> Fingerprint {
    let metadata = std::fs::symlink_metadata(path).unwrap();
    Fingerprint {
        bytes: std::fs::read(path).unwrap(),
        len: metadata.len(),
        modified: metadata.modified().unwrap(),
        readonly: metadata.permissions().readonly(),
        #[cfg(unix)]
        mode: {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode()
        },
    }
}

#[test]
fn roots_use_each_xdg_override_or_the_documented_home_default() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().join("home");
    let defaults = StartupPaths::from_environment(&environment(&home)).unwrap();
    assert_eq!(defaults.config(), home.join(".config/pi"));
    assert_eq!(defaults.data(), home.join(".local/share/pi"));
    assert_eq!(defaults.state(), home.join(".local/state/pi"));
    assert_eq!(defaults.cache(), home.join(".cache/pi"));
    assert_eq!(defaults.legacy(), home.join(".pi/agent"));

    let cases = [
        ("XDG_CONFIG_HOME", Resource::Config, "config-home"),
        ("XDG_DATA_HOME", Resource::Packages, "data-home"),
        ("XDG_STATE_HOME", Resource::Credentials, "state-home"),
        ("XDG_CACHE_HOME", Resource::Cache, "cache-home"),
    ];
    for (variable, resource, directory) in cases {
        let override_root = scratch.path().join(directory);
        let mut values = environment(&home);
        values.insert(
            OsString::from(variable),
            override_root.as_os_str().to_owned(),
        );
        let overridden = StartupPaths::from_environment(&values).unwrap();
        let actual = match resource {
            Resource::Config => overridden.config(),
            Resource::Packages => overridden.data(),
            Resource::Credentials => overridden.state(),
            Resource::Cache => overridden.cache(),
            Resource::Sessions => unreachable!(),
        };
        assert_eq!(actual, override_root.join("pi"), "{variable}");
    }

    let mut empty = environment(&home);
    for variable in [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
    ] {
        empty.insert(OsString::from(variable), OsString::new());
    }
    assert_eq!(StartupPaths::from_environment(&empty).unwrap(), defaults);
}

#[test]
fn missing_empty_relative_and_non_unicode_environment_is_rejected() {
    let missing = StartupEnvironment::new();
    assert!(matches!(
        StartupPaths::from_environment(&missing),
        Err(StartupPathError::Missing { variable: "HOME" })
    ));

    let mut empty_home = StartupEnvironment::new();
    empty_home.insert(OsString::from("HOME"), OsString::new());
    assert!(matches!(
        StartupPaths::from_environment(&empty_home),
        Err(StartupPathError::Missing { variable: "HOME" })
    ));

    let mut relative_home = StartupEnvironment::new();
    relative_home.insert(OsString::from("HOME"), OsString::from("relative"));
    assert!(matches!(
        StartupPaths::from_environment(&relative_home),
        Err(StartupPathError::Relative {
            variable: "HOME",
            ..
        })
    ));

    let scratch = tempfile::tempdir().unwrap();
    let all_xdg_without_home = [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
    ]
    .into_iter()
    .map(|variable| {
        (
            OsString::from(variable),
            scratch.path().join(variable).into_os_string(),
        )
    })
    .collect();
    assert!(matches!(
        StartupPaths::from_environment(&all_xdg_without_home),
        Err(StartupPathError::Missing { variable: "HOME" })
    ));

    for variable in [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
    ] {
        let mut values = environment(scratch.path());
        values.insert(OsString::from(variable), OsString::from("relative"));
        assert!(matches!(
            StartupPaths::from_environment(&values),
            Err(StartupPathError::Relative { variable: found, .. }) if found == variable
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut values = environment(scratch.path());
        values.insert(
            OsString::from("XDG_CONFIG_HOME"),
            OsString::from_vec(vec![b'/', 0xff]),
        );
        assert!(matches!(
            StartupPaths::from_environment(&values),
            Err(StartupPathError::NonUnicode {
                variable: "XDG_CONFIG_HOME"
            })
        ));
    }
}

#[test]
fn every_resource_obeys_canonical_precedence_fallback_and_absence() {
    for resource in Resource::ALL {
        let scratch = tempfile::tempdir().unwrap();
        let paths = paths(scratch.path());
        let canonical = paths.canonical_resource(resource);
        let legacy = paths.legacy_resource(resource);
        create_entry(&legacy, b"legacy");
        create_entry(&canonical, b"canonical");

        let context = StartupContext::from_paths(paths.clone());
        let resolution = context.resource(resource);
        assert_eq!(resolution.source(), ResourceSource::Canonical);
        assert_eq!(resolution.selected(), Some(canonical.as_path()));
        assert_eq!(resolution.destination(), canonical);
        assert_eq!(resolution.probe(), ProbeOutcome::Present);
        assert_eq!(
            resolution.import_intent().outcome(),
            ImportOutcome::CanonicalPresent
        );

        std::fs::remove_file(&canonical).unwrap();
        let fallback = StartupContext::from_paths(paths.clone());
        let resolution = fallback.resource(resource);
        assert_eq!(resolution.source(), ResourceSource::Legacy);
        assert_eq!(resolution.selected(), Some(legacy.as_path()));
        assert_eq!(resolution.destination(), canonical);
        assert_eq!(
            resolution.import_intent().outcome(),
            ImportOutcome::Available
        );

        std::fs::remove_file(&legacy).unwrap();
        let absent = StartupContext::from_paths(paths);
        let resolution = absent.resource(resource);
        assert_eq!(resolution.source(), ResourceSource::Absent);
        assert_eq!(resolution.selected(), None);
        assert_eq!(resolution.destination(), canonical);
        assert_eq!(
            resolution.import_intent().outcome(),
            ImportOutcome::SourceAbsent
        );
    }
}

#[test]
fn malformed_and_unreadable_canonical_entries_never_fall_through() {
    for resource in Resource::ALL {
        let scratch = tempfile::tempdir().unwrap();
        let paths = paths(scratch.path());
        let canonical = paths.canonical_resource(resource);
        let legacy = paths.legacy_resource(resource);
        create_entry(&canonical, &[0xff, 0x00, 0xfe]);
        create_entry(&legacy, b"valid legacy bytes");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&canonical, std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        let resolution = StartupContext::from_paths(paths.clone())
            .resource(resource)
            .clone();
        assert_eq!(resolution.source(), ResourceSource::Canonical);
        assert_eq!(resolution.selected(), Some(canonical.as_path()));
        assert_eq!(resolution.probe(), ProbeOutcome::Present);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&canonical, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        std::fs::remove_file(&canonical).unwrap();
        std::fs::create_dir(&canonical).unwrap();
        let malformed_type = StartupContext::from_paths(paths);
        assert_eq!(
            malformed_type.resource(resource).source(),
            ResourceSource::Canonical
        );
    }
}

#[cfg(unix)]
#[test]
fn permission_denied_canonical_probe_is_not_treated_as_absent() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = tempfile::tempdir().unwrap();
    let paths = paths(scratch.path());
    let canonical = paths.canonical_resource(Resource::Credentials);
    let canonical_parent = canonical.parent().unwrap();
    let legacy = paths.legacy_resource(Resource::Credentials);
    create_entry(&legacy, b"legacy credentials");
    std::fs::create_dir_all(canonical_parent).unwrap();
    std::fs::set_permissions(canonical_parent, std::fs::Permissions::from_mode(0o000)).unwrap();

    let resolution = StartupContext::from_paths(paths.clone())
        .resource(Resource::Credentials)
        .clone();

    std::fs::set_permissions(canonical_parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(resolution.source(), ResourceSource::Canonical);
    assert_eq!(resolution.selected(), Some(canonical.as_path()));
    assert_eq!(resolution.probe(), ProbeOutcome::Inaccessible);
    assert!(resolution.diagnostic().is_some());
}

#[cfg(unix)]
#[test]
fn canonical_symlinks_and_dangling_symlinks_both_win() {
    use std::os::unix::fs::symlink;

    for resource in Resource::ALL {
        let scratch = tempfile::tempdir().unwrap();
        let paths = paths(scratch.path());
        let canonical = paths.canonical_resource(resource);
        let legacy = paths.legacy_resource(resource);
        let target = scratch.path().join("target");
        create_entry(&target, b"target");
        create_entry(&legacy, b"legacy");
        std::fs::create_dir_all(canonical.parent().unwrap()).unwrap();
        symlink(&target, &canonical).unwrap();
        assert_eq!(
            StartupContext::from_paths(paths.clone())
                .resource(resource)
                .source(),
            ResourceSource::Canonical
        );

        std::fs::remove_file(&canonical).unwrap();
        symlink(scratch.path().join("missing-target"), &canonical).unwrap();
        let dangling = StartupContext::from_paths(paths);
        assert_eq!(
            dangling.resource(resource).source(),
            ResourceSource::Canonical
        );
        assert_eq!(dangling.resource(resource).probe(), ProbeOutcome::Present);
    }
}

#[test]
fn startup_and_import_diagnostics_leave_all_legacy_bytes_and_metadata_stable() {
    let scratch = tempfile::tempdir().unwrap();
    let paths = paths(scratch.path());
    let mut before = Vec::new();
    for (index, resource) in Resource::ALL.into_iter().enumerate() {
        let legacy = paths.legacy_resource(resource);
        create_entry(&legacy, format!("legacy-{index}").as_bytes());
        before.push((legacy.clone(), fingerprint(&legacy)));
    }

    for _ in 0..2 {
        let context = StartupContext::from_paths(paths.clone());
        for resource in Resource::ALL {
            let resolution = context.resource(resource);
            let intent = resolution.import_intent();
            assert_eq!(resolution.source(), ResourceSource::Legacy);
            assert_eq!(intent.source(), paths.legacy_resource(resource));
            assert_eq!(intent.destination(), paths.canonical_resource(resource));
            assert_eq!(intent.provenance(), ResourceSource::Legacy);
            assert_eq!(intent.outcome(), ImportOutcome::Available);
            assert!(intent.diagnostic().contains("no bytes were copied"));
            assert!(!intent.destination().exists());
        }
    }

    for (legacy, original) in before {
        assert_eq!(fingerprint(&legacy), original, "{}", legacy.display());
    }
    for root in [paths.config(), paths.data(), paths.state(), paths.cache()] {
        assert!(!root.exists(), "startup created {}", root.display());
    }
}

#[test]
fn file_backed_application_observes_immutable_startup_storage_without_legacy_writes() {
    let scratch = tempfile::tempdir().unwrap();
    let home = scratch.path().join("home");
    let package_root = scratch.path().join("application");
    std::fs::create_dir_all(&package_root).unwrap();

    let overrides = [
        ("XDG_CONFIG_HOME", scratch.path().join("xdg-config")),
        ("XDG_DATA_HOME", scratch.path().join("xdg-data")),
        ("XDG_STATE_HOME", scratch.path().join("xdg-state")),
        ("XDG_CACHE_HOME", scratch.path().join("xdg-cache")),
    ];
    let mut values = environment(&home);
    for (name, value) in &overrides {
        values.insert(OsString::from(*name), value.as_os_str().to_owned());
    }
    let paths = StartupPaths::from_environment(&values).unwrap();
    let mut legacy_before = Vec::new();
    for (index, resource) in Resource::ALL.into_iter().enumerate() {
        let legacy = paths.legacy_resource(resource);
        create_entry(&legacy, format!("launcher-legacy-{index}").as_bytes());
        legacy_before.push((legacy.clone(), fingerprint(&legacy)));
    }

    let application = package_root.join("application.lua");
    std::fs::write(
        &application,
        r#"
local k = (...).kernel.v1
k.root({
  kind="application", id="startup-path-observer", active=true, priority=0,
  dispatch=function(snapshot)
    local storage = snapshot.context.storage
    local immutable = not pcall(function() storage.paths.config = "changed" end)
    local resources = {}
    for index = 1, #storage.resources do
      local resource = storage.resources[index]
      resources[index] = {
        resource=resource.resource,
        source=resource.source,
        selected=resource.selected,
        destination=resource.destination,
      }
    end
    k.action("observed_startup_storage", {
      immutable=immutable,
      paths={
        config=storage.paths.config,
        data=storage.paths.data,
        state=storage.paths.state,
        cache=storage.paths.cache,
        legacy=storage.paths.legacy,
      },
      resources=resources,
    })
  end,
})
"#,
    )
    .unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_pi"));
    command
        .current_dir(&package_root)
        .arg("--package")
        .arg("application.lua")
        .env("HOME", &home);
    for (name, value) in &overrides {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let observed = &output["actions"][0]["payload"];
    assert_eq!(observed["immutable"], true);
    assert_eq!(
        observed["paths"]["config"],
        paths.config().to_string_lossy().as_ref()
    );
    assert_eq!(
        observed["paths"]["data"],
        paths.data().to_string_lossy().as_ref()
    );
    assert_eq!(
        observed["paths"]["state"],
        paths.state().to_string_lossy().as_ref()
    );
    assert_eq!(
        observed["paths"]["cache"],
        paths.cache().to_string_lossy().as_ref()
    );
    assert_eq!(observed["resources"].as_array().unwrap().len(), 5);
    assert!(
        observed["resources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|resource| resource["source"] == "legacy")
    );

    for (legacy, original) in legacy_before {
        assert_eq!(fingerprint(&legacy), original, "{}", legacy.display());
    }
    for resource in Resource::ALL {
        assert!(!paths.canonical_resource(resource).exists());
    }
}

#[test]
fn launcher_rejects_missing_home_before_using_the_working_directory() {
    let scratch = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_pi"))
        .current_dir(scratch.path())
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CACHE_HOME")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "pi: cannot derive application startup paths: HOME is required and must not be empty\n"
    );
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
}
