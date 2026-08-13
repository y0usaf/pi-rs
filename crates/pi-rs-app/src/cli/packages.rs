//! Port of `package-manager-cli.ts` — the `pi install/remove/update/list`
//! (and uninstall alias) CLI surface.
//!
//! This module owns the parse + help + early-error surface, which returns
//! before any settings/trust/package-manager/network work. The observable
//! outcome for those hermetic cases is pinned byte-for-byte by the
//! differential in `tests/package-cli-parity` (a Pi-generated oracle from
//! `handlePackageCommand`, driven with Bun). The full install/remove/update/
//! list execution (settings manager, trust, npm/git/self-update) lives in the
//! Lua `pi.packages` module and Rust dispatch completes it; this fixture's
//! scope is exactly the deterministic prefix.
//!
//! Usage/help text is reproduced verbatim from the spec (with `APP_NAME`,
//! matching the args.rs help precedent).

/// Spec: `PackageCommand`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PackageCommand {
    #[default]
    Install,
    Remove,
    Update,
    List,
}

impl PackageCommand {
    pub fn as_str(self) -> &'static str {
        match self {
            PackageCommand::Install => "install",
            PackageCommand::Remove => "remove",
            PackageCommand::Update => "update",
            PackageCommand::List => "list",
        }
    }
}

/// Spec: `UpdateTarget`.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateTarget {
    All,
    UpdateSelf,
    Extensions { source: Option<String> },
}

/// Spec: `PackageCommandOptions`.
#[derive(Debug, Clone, Default)]
pub struct PackageCommandOptions {
    pub command: PackageCommand,
    pub source: Option<String>,
    pub update_target: Option<UpdateTarget>,
    pub local: bool,
    pub force: bool,
    pub project_trust_override: Option<bool>,
    pub help: bool,
    pub invalid_option: Option<String>,
    pub invalid_argument: Option<String>,
    pub missing_option_value: Option<String>,
    pub conflicting_options: Option<String>,
}

/// Usage strings mirror Pi's `getPackageCommandUsage`.
pub fn package_command_usage(command: PackageCommand) -> &'static str {
    match command {
        PackageCommand::Install => "pi install <source> [-l] [--approve|--no-approve]",
        PackageCommand::Remove => "pi remove <source> [-l] [--approve|--no-approve]",
        PackageCommand::Update => {
            "pi update [source|self|pi] [--self] [--extensions] [--extension <source>] [--approve|--no-approve] [--force]"
        }
        PackageCommand::List => "pi list [--approve|--no-approve]",
    }
}

/// Help text mirrors Pi's `printPackageCommandHelp` (chalk-stripped).
pub fn package_command_help(command: PackageCommand) -> String {
    match command {
        PackageCommand::Install => {
            "Usage:\n  pi install <source> [-l] [--approve|--no-approve]\n\nInstall a package and add it to settings.\n\nOptions:\n  -l, --local       Install project-locally (.pi/settings.json)\n  -a, --approve     Trust project-local files for this command\n  -na, --no-approve Ignore project-local files for this command\n\nExamples:\n  pi install npm:@foo/bar\n  pi install git:github.com/user/repo\n  pi install git:git@github.com:user/repo\n  pi install https://github.com/user/repo\n  pi install ssh://git@github.com/user/repo\n  pi install ./local/path\n\n"
                .to_owned()
        }
        PackageCommand::Remove => {
            "Usage:\n  pi remove <source> [-l] [--approve|--no-approve]\n\nRemove a package and its source from settings.\nAlias: pi uninstall <source> [-l]\n\nOptions:\n  -l, --local       Remove from project settings (.pi/settings.json)\n  -a, --approve     Trust project-local files for this command\n  -na, --no-approve Ignore project-local files for this command\n\nExamples:\n  pi remove npm:@foo/bar\n  pi uninstall npm:@foo/bar\n\n"
                .to_owned()
        }
        PackageCommand::Update => {
            "Usage:\n  pi update [source|self|pi] [--self] [--extensions] [--extension <source>] [--approve|--no-approve] [--force]\n\nUpdate pi and installed packages.\n\nOptions:\n  --self                  Update pi only\n  --extensions            Update installed packages only\n  --extension <source>    Update one package only\n  -a, --approve           Trust project-local files for this command\n  -na, --no-approve       Ignore project-local files for this command\n  --force                 Reinstall pi even if the current version is latest\n\nShort forms:\n  pi update                Update pi and all extensions\n  pi update <source>       Update one package\n  pi update pi             Update pi only (self works as alias to pi)\n\n"
                .to_owned()
        }
        PackageCommand::List => {
            "Usage:\n  pi list [--approve|--no-approve]\n\nList installed packages from user and project settings.\n\nOptions:\n  -a, --approve      Trust project-local files for this command\n  -na, --no-approve  Ignore project-local files for this command\n\n"
                .to_owned()
        }
    }
}

/// Spec: `parsePackageCommand(args)` — returns `None` when the leading token
/// is not a package command (so the CLI treats the arg as a normal prompt).
pub fn parse_package_command(args: &[String]) -> Option<PackageCommandOptions> {
    let [raw_command, rest @ ..] = args else {
        return None;
    };

    let command = match raw_command.as_str() {
        "uninstall" => PackageCommand::Remove,
        "install" => PackageCommand::Install,
        "remove" => PackageCommand::Remove,
        "update" => PackageCommand::Update,
        "list" => PackageCommand::List,
        _ => return None,
    };

    let mut options = PackageCommandOptions {
        command,
        ..Default::default()
    };

    let mut self_flag = false;
    let mut extensions_flag = false;
    let mut extension_flag_source: Option<String> = None;

    let mut index = 0usize;
    while index < rest.len() {
        let arg = &rest[index];
        if arg == "-h" || arg == "--help" {
            options.help = true;
            index += 1;
            continue;
        }
        if arg == "-l" || arg == "--local" {
            if matches!(command, PackageCommand::Install | PackageCommand::Remove) {
                options.local = true;
            } else if options.invalid_option.is_none() {
                options.invalid_option = Some(arg.clone());
            }
            index += 1;
            continue;
        }
        if arg == "--self" {
            if command == PackageCommand::Update {
                self_flag = true;
            } else if options.invalid_option.is_none() {
                options.invalid_option = Some(arg.clone());
            }
            index += 1;
            continue;
        }
        if arg == "--extensions" {
            if command == PackageCommand::Update {
                extensions_flag = true;
            } else if options.invalid_option.is_none() {
                options.invalid_option = Some(arg.clone());
            }
            index += 1;
            continue;
        }
        if arg == "--approve" || arg == "-a" {
            options.project_trust_override = Some(true);
            index += 1;
            continue;
        }
        if arg == "--no-approve" || arg == "-na" {
            options.project_trust_override = Some(false);
            index += 1;
            continue;
        }
        if arg == "--force" {
            if command == PackageCommand::Update {
                options.force = true;
            } else if options.invalid_option.is_none() {
                options.invalid_option = Some(arg.clone());
            }
            index += 1;
            continue;
        }
        if arg == "--extension" {
            if command != PackageCommand::Update {
                if options.invalid_option.is_none() {
                    options.invalid_option = Some(arg.clone());
                }
                index += 1;
                continue;
            }
            let value = rest.get(index + 1);
            match value {
                None => {
                    if options.missing_option_value.is_none() {
                        options.missing_option_value = Some(arg.clone());
                    }
                    index += 1;
                }
                Some(value) if value.starts_with('-') => {
                    if options.missing_option_value.is_none() {
                        options.missing_option_value = Some(arg.clone());
                    }
                    index += 1;
                }
                Some(value) => {
                    if extension_flag_source.is_some() {
                        if options.conflicting_options.is_none() {
                            options.conflicting_options =
                                Some("--extension can only be provided once".to_owned());
                        }
                        index += 2; // skip --extension + its value token
                    } else {
                        extension_flag_source = Some(value.clone());
                        index += 2; // skip --extension + its value token
                    }
                }
            }
            continue;
        }
        if arg.starts_with('-') {
            if options.invalid_option.is_none() {
                options.invalid_option = Some(arg.clone());
            }
            index += 1;
            continue;
        }
        if options.source.is_none() {
            options.source = Some(arg.clone());
        } else if options.invalid_argument.is_none() {
            options.invalid_argument = Some(arg.clone());
        }
        index += 1;
    }

    if command == PackageCommand::Update {
        if let Some(extension_source) = &extension_flag_source {
            if (self_flag || extensions_flag) && options.conflicting_options.is_none() {
                options.conflicting_options =
                    Some("--extension cannot be combined with --self or --extensions".to_owned());
            }
            if options.source.is_some() && options.conflicting_options.is_none() {
                options.conflicting_options =
                    Some("--extension cannot be combined with a positional source".to_owned());
            }
            options.update_target = Some(UpdateTarget::Extensions {
                source: Some(extension_source.clone()),
            });
        } else if let Some(source) = &options.source {
            let source_is_self = source == "self" || source == "pi";
            if source_is_self {
                options.update_target = Some(if extensions_flag {
                    UpdateTarget::All
                } else {
                    UpdateTarget::UpdateSelf
                });
            } else {
                if (extensions_flag || self_flag) && options.conflicting_options.is_none() {
                    options.conflicting_options = Some(
                        "positional update targets cannot be combined with --self or --extensions"
                            .to_owned(),
                    );
                }
                options.update_target = Some(UpdateTarget::Extensions {
                    source: Some(source.clone()),
                });
            }
        } else if self_flag && extensions_flag {
            options.update_target = Some(UpdateTarget::All);
        } else if self_flag {
            options.update_target = Some(UpdateTarget::UpdateSelf);
        } else if extensions_flag {
            options.update_target = Some(UpdateTarget::Extensions { source: None });
        } else {
            options.update_target = Some(UpdateTarget::All);
        }
    }

    Some(options)
}

/// Whether the target includes the self-update leg (spec `updateTargetIncludesSelf`).
pub fn update_target_includes_self(target: &UpdateTarget) -> bool {
    matches!(target, UpdateTarget::All | UpdateTarget::UpdateSelf)
}

/// Whether the target includes the extensions leg (spec `updateTargetIncludesExtensions`).
pub fn update_target_includes_extensions(target: &UpdateTarget) -> bool {
    matches!(target, UpdateTarget::All | UpdateTarget::Extensions { .. })
}

/// The deterministic early-return outcome of `handlePackageCommand` for the
/// Hermetic parse/help/error surface: `None` (not a package command) or
/// `Some((exit_code, stdout, stderr))`. Commands that would proceed to
/// settings/trust/network work are out of this fixture's scope and return
/// `Some((i32::MIN, String::new(), String::new()))` as a sentinel, so
/// the differential never captures non-deterministic execution.
pub fn handle_package_command_hermetic(args: &[String]) -> Option<(i32, String, String)> {
    let options = parse_package_command(args)?;

    if options.help {
        return Some((0, package_command_help(options.command), String::new()));
    }
    if let Some(flag) = &options.invalid_option {
        let stderr = format!(
            "Unknown option {flag} for \"{}\".\nUse \"pi --help\" or \"{}\".\n",
            options.command.as_str(),
            package_command_usage(options.command)
        );
        return Some((1, String::new(), stderr));
    }
    if let Some(flag) = &options.missing_option_value {
        let stderr = format!(
            "Missing value for {flag}.\nUsage: {}\n",
            package_command_usage(options.command)
        );
        return Some((1, String::new(), stderr));
    }
    if let Some(arg) = &options.invalid_argument {
        let stderr = format!(
            "Unexpected argument {arg}.\nUsage: {}\n",
            package_command_usage(options.command)
        );
        return Some((1, String::new(), stderr));
    }
    if let Some(message) = &options.conflicting_options {
        let stderr = format!(
            "{message}\nUsage: {}\n",
            package_command_usage(options.command)
        );
        return Some((1, String::new(), stderr));
    }

    let source = options.source.clone();
    if matches!(
        options.command,
        PackageCommand::Install | PackageCommand::Remove
    ) && source.is_none()
    {
        let stderr = format!(
            "Missing {} source.\nUsage: {}\n",
            options.command.as_str(),
            package_command_usage(options.command)
        );
        return Some((1, String::new(), stderr));
    }

    // A command that proceeds to settings/trust/package-manager work (local
    // install/remove/list with settings, or update). This is out of the
    // hermetic scope; the sentinel marks it as "would execute".
    Some((i32::MIN, String::new(), String::new()))
}

/// Build the `pkg-exec` role request from parsed package command options. The
/// Lua role (`pi.packages`) runs the deterministic execution legs through the
/// shared module mechanism and returns Pi-matching stdout/stderr/exitCode.
pub fn package_exec_request(
    options: &PackageCommandOptions,
    cwd: String,
    agent_dir: String,
) -> serde_json::Value {
    serde_json::json!({
        "command": options.command.as_str(),
        "source": options.source,
        "local_scope": options.local,
        "cwd": cwd,
        "agentDir": agent_dir,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;

    fn parse(args: &[&str]) -> PackageCommandOptions {
        parse_package_command(&args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
            .expect("package command")
    }

    #[test]
    fn non_command_returns_none() {
        assert!(parse_package_command(&["echo".into(), "hi".into()]).is_none());
        assert!(parse_package_command(&["bogus".into()]).is_none());
        assert!(parse_package_command(&[].to_vec()).is_none());
    }

    #[test]
    fn uninstall_aliases_to_remove() {
        assert_eq!(
            parse(&["uninstall", "npm:x"]).command,
            PackageCommand::Remove
        );
    }

    #[test]
    fn local_flag_narrowly_scoped() {
        // -l is valid for install/remove only.
        assert!(parse(&["install", "./p", "-l"]).local);
        assert!(parse(&["remove", "./p", "--local"]).local);
        let list = parse(&["list", "--local"]);
        assert_eq!(list.invalid_option.as_deref(), Some("--local"));
    }

    #[test]
    fn update_targets() {
        assert_eq!(parse(&["update"]).update_target, Some(UpdateTarget::All));
        assert_eq!(
            parse(&["update", "self"]).update_target,
            Some(UpdateTarget::UpdateSelf)
        );
        assert_eq!(
            parse(&["update", "pi"]).update_target,
            Some(UpdateTarget::UpdateSelf)
        );
        assert_eq!(
            parse(&["update", "--extensions"]).update_target,
            Some(UpdateTarget::Extensions { source: None })
        );
        assert_eq!(
            parse(&["update", "--extension", "npm:a/b"]).update_target,
            Some(UpdateTarget::Extensions {
                source: Some("npm:a/b".to_owned())
            })
        );
        // --self --extensions -> all
        assert_eq!(
            parse(&["update", "--self", "--extensions"]).update_target,
            Some(UpdateTarget::All)
        );
    }

    #[test]
    fn hermetic_outcomes_match_oracle_shape() {
        let cases: Vec<(&[&str], bool)> = vec![
            (&["install", "--help"], true),
            (&["echo", "hi"], false),
            (&["install"], true),
            (&["list", "--local"], true),
            (&["install", "a", "b"], true),
            (&["update", "foo", "--self"], true),
        ];
        for (argv, expected_handled) in cases {
            let handled =
                parse_package_command(&argv.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
                    .is_some();
            assert_eq!(handled, expected_handled, "argv: {argv:?}");
        }
    }

    #[test]
    fn help_text_matches_spec() {
        assert_eq!(
            package_command_usage(PackageCommand::Install),
            "pi install <source> [-l] [--approve|--no-approve]"
        );
        assert!(package_command_help(PackageCommand::List).starts_with("Usage:\n  pi list"));
    }
}
