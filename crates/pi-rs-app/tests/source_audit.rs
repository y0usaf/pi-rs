#![allow(clippy::expect_used)]

use std::path::PathBuf;

#[test]
fn launcher_sources_do_not_link_embedded_lua_or_product_branches() {
    let manifest =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("read app manifest");
    let runtime = manifest
        .split_once("[dependencies]")
        .and_then(|(_, rest)| rest.split_once("[dev-dependencies]"))
        .map(|(dependencies, _)| dependencies)
        .expect("runtime dependencies section");
    for product_crate in [
        "pi-rs-agent",
        "pi-rs-ai =",
        "pi-rs-ai-auth",
        "pi-rs-session",
    ] {
        assert!(
            !runtime.contains(product_crate),
            "unexpected {product_crate}"
        );
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut pending = vec![root];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(path).expect("read source directory") {
            let entry = entry.expect("source entry");
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).expect("read Rust source");
                assert!(!source.contains("include_str!("), "{}", path.display());
                assert!(
                    !source.contains("PackageSource::Embedded"),
                    "{}",
                    path.display()
                );
            }
        }
    }
}
