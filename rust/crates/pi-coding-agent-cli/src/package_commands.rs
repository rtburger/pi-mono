use crate::package_manager::{
    DefaultPackageManager, ResourceScope, ResolvedResource,
};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageCommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageCommand {
    Install,
    Remove,
    Update,
    List,
    Config,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackageCommandOptions {
    command: PackageCommand,
    source: Option<String>,
    local: bool,
    help: bool,
    invalid_option: Option<String>,
}

pub fn handle_package_or_config_command(
    args: &[String],
    cwd: &Path,
    agent_dir: &Path,
) -> Option<PackageCommandOutput> {
    let options = parse_package_command(args)?;
    let manager = DefaultPackageManager::new(cwd.to_path_buf(), agent_dir.to_path_buf());

    let mut stderr = render_settings_warnings(&manager, command_context_label(options.command));
    let mut stdout = String::new();

    if options.help {
        stdout.push_str(&package_command_help(options.command));
        return Some(PackageCommandOutput {
            stdout,
            stderr,
            exit_code: 0,
        });
    }

    if let Some(invalid_option) = options.invalid_option.as_ref() {
        stderr.push_str(&format!(
            "Unknown option {invalid_option} for \"{}\".\n",
            command_name(options.command)
        ));
        stderr.push_str(&format!(
            "Use \"{}\".\n",
            package_command_usage(options.command)
        ));
        return Some(PackageCommandOutput {
            stdout,
            stderr,
            exit_code: 1,
        });
    }

    if matches!(options.command, PackageCommand::Install | PackageCommand::Remove) && options.source.is_none() {
        stderr.push_str(&format!(
            "Missing {} source.\n",
            command_name(options.command)
        ));
        stderr.push_str(&format!(
            "Usage: {}\n",
            package_command_usage(options.command)
        ));
        return Some(PackageCommandOutput {
            stdout,
            stderr,
            exit_code: 1,
        });
    }

    let result = match options.command {
        PackageCommand::Install => {
            let source = options.source.as_deref().expect("validated source");
            manager.install_and_persist(source, options.local).map(|_| {
                format!("Installed {source}\n")
            })
        }
        PackageCommand::Remove => {
            let source = options.source.as_deref().expect("validated source");
            match manager.remove_and_persist(source, options.local) {
                Ok(true) => Ok(format!("Removed {source}\n")),
                Ok(false) => Err(format!("No matching package found for {source}")),
                Err(error) => Err(error),
            }
        }
        PackageCommand::Update => match manager.update(options.source.as_deref()) {
            Ok(()) => Ok(match options.source.as_deref() {
                Some(source) => format!("Updated {source}\n"),
                None => String::from("Updated packages\n"),
            }),
            Err(error) => Err(error),
        },
        PackageCommand::List => Ok(render_package_list(&manager)),
        PackageCommand::Config => manager.resolve().map(|resolved| render_config_summary(&resolved.resolved)),
    };

    match result {
        Ok(output) => {
            stdout.push_str(&output);
            Some(PackageCommandOutput {
                stdout,
                stderr,
                exit_code: 0,
            })
        }
        Err(error) => {
            stderr.push_str(&format!("Error: {error}\n"));
            Some(PackageCommandOutput {
                stdout,
                stderr,
                exit_code: 1,
            })
        }
    }
}

fn parse_package_command(args: &[String]) -> Option<PackageCommandOptions> {
    let (raw_command, rest) = args.split_first()?;
    let command = match raw_command.as_str() {
        "install" => PackageCommand::Install,
        "remove" | "uninstall" => PackageCommand::Remove,
        "update" => PackageCommand::Update,
        "list" => PackageCommand::List,
        "config" => PackageCommand::Config,
        _ => return None,
    };

    let mut source = None;
    let mut local = false;
    let mut help = false;
    let mut invalid_option = None;

    for arg in rest {
        match arg.as_str() {
            "-h" | "--help" => help = true,
            "-l" | "--local" => {
                if matches!(command, PackageCommand::Install | PackageCommand::Remove) {
                    local = true;
                } else {
                    invalid_option.get_or_insert_with(|| arg.clone());
                }
            }
            value if value.starts_with('-') => {
                invalid_option.get_or_insert_with(|| arg.clone());
            }
            _ => {
                if source.is_none() {
                    source = Some(arg.clone());
                }
            }
        }
    }

    Some(PackageCommandOptions {
        command,
        source,
        local,
        help,
        invalid_option,
    })
}

fn render_settings_warnings(manager: &DefaultPackageManager, context: &str) -> String {
    manager
        .settings_warnings()
        .into_iter()
        .map(|warning| {
            format!(
                "Warning ({context}, {} settings): {}\n",
                warning.scope.label(),
                warning.message
            )
        })
        .collect::<String>()
}

fn render_package_list(manager: &DefaultPackageManager) -> String {
    let packages = manager.list_configured_packages();
    if packages.is_empty() {
        return String::from("No packages installed.\n");
    }

    let mut output = String::new();
    let user_packages = packages
        .iter()
        .filter(|package| package.scope == ResourceScope::User)
        .collect::<Vec<_>>();
    let project_packages = packages
        .iter()
        .filter(|package| package.scope == ResourceScope::Project)
        .collect::<Vec<_>>();

    if !user_packages.is_empty() {
        output.push_str("User packages:\n");
        for package in user_packages {
            push_package_line(&mut output, package.source.as_str(), package.filtered, package.installed_path.as_deref());
        }
    }
    if !project_packages.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str("Project packages:\n");
        for package in project_packages {
            push_package_line(&mut output, package.source.as_str(), package.filtered, package.installed_path.as_deref());
        }
    }

    output
}

fn push_package_line(output: &mut String, source: &str, filtered: bool, installed_path: Option<&str>) {
    if filtered {
        output.push_str(&format!("  {source} (filtered)\n"));
    } else {
        output.push_str(&format!("  {source}\n"));
    }
    if let Some(installed_path) = installed_path {
        output.push_str(&format!("    {installed_path}\n"));
    }
}

fn render_config_summary(resolved: &crate::package_manager::ResolvedPaths) -> String {
    let mut output = String::new();
    render_resource_group(&mut output, "Extensions", &resolved.extensions);
    render_resource_group(&mut output, "Skills", &resolved.skills);
    render_resource_group(&mut output, "Prompt templates", &resolved.prompts);
    render_resource_group(&mut output, "Themes", &resolved.themes);
    if output.is_empty() {
        String::from("No resources found.\n")
    } else {
        output
    }
}

fn render_resource_group(output: &mut String, title: &str, resources: &[ResolvedResource]) {
    if resources.is_empty() {
        return;
    }
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(title);
    output.push_str(":\n");
    for resource in resources {
        let enabled = if resource.enabled { "[x]" } else { "[ ]" };
        output.push_str(&format!(
            "  {enabled} {} ({}, {})\n",
            resource.path,
            resource.metadata.scope.as_str(),
            resource.metadata.source,
        ));
    }
}

fn package_command_usage(command: PackageCommand) -> String {
    match command {
        PackageCommand::Install => String::from("pi install <source> [-l]"),
        PackageCommand::Remove => String::from("pi remove <source> [-l]"),
        PackageCommand::Update => String::from("pi update [source]"),
        PackageCommand::List => String::from("pi list"),
        PackageCommand::Config => String::from("pi config"),
    }
}

fn package_command_help(command: PackageCommand) -> String {
    match command {
        PackageCommand::Install => String::from(
            "Usage:\n  pi install <source> [-l]\n\nInstall a package and add it to settings.\n\nOptions:\n  -l, --local    Install project-locally (.pi/settings.json)\n",
        ),
        PackageCommand::Remove => String::from(
            "Usage:\n  pi remove <source> [-l]\n\nRemove a package and its source from settings.\nAlias: pi uninstall <source> [-l]\n\nOptions:\n  -l, --local    Remove from project settings (.pi/settings.json)\n",
        ),
        PackageCommand::Update => String::from(
            "Usage:\n  pi update [source]\n\nUpdate installed packages. If <source> is provided, only that package is updated.\n",
        ),
        PackageCommand::List => {
            String::from("Usage:\n  pi list\n\nList installed packages from user and project settings.\n")
        }
        PackageCommand::Config => String::from(
            "Usage:\n  pi config\n\nShow resolved extensions, skills, prompt templates, and themes.\n",
        ),
    }
}

fn command_name(command: PackageCommand) -> &'static str {
    match command {
        PackageCommand::Install => "install",
        PackageCommand::Remove => "remove",
        PackageCommand::Update => "update",
        PackageCommand::List => "list",
        PackageCommand::Config => "config",
    }
}

fn command_context_label(command: PackageCommand) -> &'static str {
    match command {
        PackageCommand::Config => "config command",
        _ => "package command",
    }
}

#[cfg(test)]
mod tests {
    use super::handle_package_or_config_command;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pi-coding-agent-cli-package-commands-{prefix}-{unique}"
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn installs_lists_and_removes_local_packages() {
        let temp_dir = unique_temp_dir("local");
        let cwd = temp_dir.join("project");
        let agent_dir = temp_dir.join("agent");
        let package_dir = cwd.join("packages").join("demo");
        fs::create_dir_all(package_dir.join("extensions")).unwrap();
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(package_dir.join("extensions").join("index.ts"), "export default function () {}\n").unwrap();

        let install = handle_package_or_config_command(
            &[String::from("install"), String::from("./packages/demo")],
            &cwd,
            &agent_dir,
        )
        .expect("expected install command to be handled");
        assert_eq!(install.exit_code, 0, "stderr: {}", install.stderr);
        assert!(install.stdout.contains("Installed ./packages/demo"));

        let listed = handle_package_or_config_command(&[String::from("list")], &cwd, &agent_dir)
            .expect("expected list command to be handled");
        assert_eq!(listed.exit_code, 0, "stderr: {}", listed.stderr);
        assert!(listed.stdout.contains("User packages:"), "stdout: {}", listed.stdout);
        assert!(listed.stdout.contains("../project/packages/demo"), "stdout: {}", listed.stdout);

        let removed = handle_package_or_config_command(
            &[String::from("remove"), package_dir.to_string_lossy().into_owned()],
            &cwd,
            &agent_dir,
        )
        .expect("expected remove command to be handled");
        assert_eq!(removed.exit_code, 0, "stderr: {}", removed.stderr);
        assert!(removed.stdout.contains("Removed"), "stdout: {}", removed.stdout);
    }

    #[test]
    fn config_command_summarizes_resolved_resources() {
        let temp_dir = unique_temp_dir("config");
        let cwd = temp_dir.join("project");
        let agent_dir = temp_dir.join("agent");
        let package_dir = temp_dir.join("package");
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::create_dir_all(&agent_dir).unwrap();
        fs::create_dir_all(package_dir.join("prompts")).unwrap();
        fs::write(package_dir.join("prompts").join("review.md"), "Review\n").unwrap();
        fs::write(
            cwd.join(".pi").join("settings.json"),
            format!("{{\n  \"packages\": [\"{}\"]\n}}\n", package_dir.display()),
        )
        .unwrap();

        let output = handle_package_or_config_command(&[String::from("config")], &cwd, &agent_dir)
            .expect("expected config command to be handled");
        assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
        assert!(output.stdout.contains("Prompt templates:"), "stdout: {}", output.stdout);
        assert!(output.stdout.contains("review.md"), "stdout: {}", output.stdout);
    }
}
