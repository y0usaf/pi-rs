//! Product extension loading between startup resource resolution and mode dispatch.

use pi_rs_host::{Host, LoadReport};

/// Resolve and load ordinary Lua extensions in Pi's product precedence order.
/// Embedded packs are installed by the caller before this step.
pub fn load_product_extensions(
    host: &Host,
    configured_paths: &[String],
    cli_paths: &[String],
    cwd: &str,
    agent_dir: &str,
    project_trusted: bool,
    no_extensions: bool,
) -> LoadReport {
    let paths = pi_rs_host::discover::product_extension_paths(
        configured_paths,
        cli_paths,
        cwd,
        agent_dir,
        project_trusted,
        no_extensions,
    );
    host.load_extensions(&paths)
}
