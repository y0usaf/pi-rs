{
  description = "pi-rs — a minimal Rust harness with Lua-authored product policy";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane = {
      url = "github:ipetkov/crane";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system);
      lib = nixpkgs.lib;
      mkPkgs = system: import nixpkgs { inherit system; };

      # Shared crane setup: source filter + dependency artifacts. Checks
      # derive from the same `cargoArtifacts` so the dependency build is
      # cached across `nix flake check` invocations.
      mkCraneLib =
        system:
        let
          pkgs = mkPkgs system;
          craneLib = crane.mkLib pkgs;

          # crane's default filter strips non-Rust files; the flake must
          # see embedded packs/assets and recorded protocol fixtures — locked
          # decision: every embedded file type is in the source filter.
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter =
              path: type:
              (craneLib.filterCargoSources path type)
              || (lib.hasSuffix ".json" path)
              || (lib.hasSuffix ".lua" path)
              || (lib.hasSuffix ".html" path)
              || (lib.hasSuffix ".css" path)
              || (lib.hasSuffix ".js" path)
              || (lib.hasSuffix ".md" path)
              || (lib.hasSuffix ".base64" path)
              || (lib.hasSuffix ".hex" path)
              || (lib.hasSuffix ".pem" path)
              || (lib.hasSuffix ".sse" path);
          };

          commonEnv = {
            # Clear rustup env vars so nix's rustc-wrapper doesn't pick up
            # a rustup-managed toolchain in the sandbox.
            RUSTUP_HOME = "";
            RUSTUP_TOOLCHAIN = "";
            nativeBuildInputs = [ pkgs.llvmPackages.bintools ];
          };

          cargoArtifacts = craneLib.buildDepsOnly {
            inherit src;
            pname = "pi-rs-deps";
            version = "0.1.0";
            inherit (commonEnv) RUSTUP_HOME RUSTUP_TOOLCHAIN nativeBuildInputs;
          };
        in
        {
          inherit
            pkgs
            craneLib
            src
            cargoArtifacts
            commonEnv
            ;
        };

      # The `pi` binary (crates/pi-rs-app).
      mkPiRs =
        system:
        let
          c = mkCraneLib system;
        in
        c.craneLib.buildPackage {
          inherit (c) src cargoArtifacts;
          pname = "pi-rs";
          version = "0.1.0";
          nativeBuildInputs = c.commonEnv.nativeBuildInputs ++ [
            c.pkgs.ripgrep
            c.pkgs.fd
          ];
          cargoExtraArgs = "-p pi-rs-app";
          doCheck = false;
          meta.mainProgram = "pi";
        };

      # Installed-launcher smoke test: the packaged binary starts, diagnoses
      # missing credentials, and exposes the retained provider catalog. Actual
      # zero-pack/source-neutral boot is owned by app/tests/assembly.rs.
      mkLauncherSmoke =
        system:
        let
          pkgs = mkPkgs system;
          piRs = mkPiRs system;
        in
        pkgs.runCommand "launcher-smoke"
          {
            nativeBuildInputs = [
              piRs
              pkgs.jq
            ];
          }
          ''
            export HOME=$TMPDIR

            # --version prints the version and exits 0.
            version=$(pi --version)
            test -n "$version"

            # --help prints usage. Capture before grep: Rust's stdout panics
            # when a successful `grep -q` closes a pipe early.
            pi --help > help.txt
            grep -q -- '--list-models' help.txt

            # No credentials: --list-models reports guidance, exit 0.
            pi --list-models > no-models.txt
            grep -q 'No models available.' no-models.txt

            # No credentials: a prompt fails with the guidance, exit 1.
            if pi "hi" 2>err.txt; then
              echo 'expected `pi "hi"` to fail without credentials' >&2
              exit 1
            fi
            grep -q 'No models available.' err.txt

            # With an anthropic key: --list-models lists exactly the
            # anthropic rows of pi's catalog (WS2 acceptance).
            export ANTHROPIC_API_KEY=dummy
            pi --list-models > list.txt
            head -1 list.txt | grep -q '^provider'
            grep -q 'claude-opus-4-8' list.txt
            rows=$(($(wc -l < list.txt) - 1))
            expected=$(jq -r '.[] | select(.provider=="anthropic") | .models | length' ${./crates/pi-rs-ai/data/models.json})
            test "$rows" -eq "$expected"

            touch $out
          '';

      # `cargo test` across the whole workspace.
      mkTest =
        system:
        let
          c = mkCraneLib system;
        in
        c.craneLib.cargoTest {
          inherit (c) src cargoArtifacts;
          pname = "pi-rs-test";
          version = "0.1.0";
          nativeBuildInputs = c.commonEnv.nativeBuildInputs ++ [
            c.pkgs.ripgrep
            c.pkgs.fd
          ];
          cargoExtraArgs = "--workspace";
        };

      # Clippy with warnings denied on shipped targets. The explicitly allowed
      # style lints changed under the pinned toolchain; safety lints,
      # including unwrap/expect/panic in library code, remain denied.
      mkClippy =
        system:
        let
          c = mkCraneLib system;
        in
        c.craneLib.cargoClippy {
          inherit (c) src cargoArtifacts;
          pname = "pi-rs-clippy";
          version = "0.1.0";
          inherit (c.commonEnv) RUSTUP_HOME RUSTUP_TOOLCHAIN nativeBuildInputs;
          cargoClippyExtraArgs = "--workspace --lib --bins -- --deny warnings --allow clippy::collapsible_if --allow clippy::collapsible_match --allow clippy::needless_update";
        };

      # Offline, fixture-backed normalization and rejection tests for the
      # reviewed model-catalog update path.
      mkModelCatalogUpdateTest =
        system:
        let
          pkgs = mkPkgs system;
        in
        pkgs.runCommand "model-catalog-update-test"
          {
            nativeBuildInputs = [
              pkgs.bash
              pkgs.bun
              pkgs.jq
            ];
          }
          ''
            bash ${self}/scripts/test-model-catalog-update
            touch $out
          '';

      mkModelCatalogUpdater =
        system:
        let
          pkgs = mkPkgs system;
        in
        pkgs.writeShellApplication {
          name = "update-model-catalog";
          runtimeInputs = [
            pkgs.bun
            pkgs.git
          ];
          text = ''
            exec bun ${self}/scripts/update-model-catalog.ts "$@"
          '';
        };

      mkDemo =
        system:
        let
          pkgs = mkPkgs system;
          # nixpkgs e73de5be's libwebsockets embeds a doubled plugin path,
          # which prevents ttyd (and therefore VHS) from starting. This is
          # the upstream fix already present in newer nixpkgs revisions.
          libwebsockets = pkgs.libwebsockets.overrideAttrs (old: {
            postPatch = old.postPatch + ''
              substituteInPlace cmake/lws_config.h.in \
                --replace-fail '"''${CMAKE_INSTALL_PREFIX}/''${LWS_INSTALL_LIB_DIR}"' \
                               '"''${CMAKE_INSTALL_FULL_LIBDIR}"'
            '';
          });
          ttyd = pkgs.ttyd.override { inherit libwebsockets; };
          vhs = pkgs.vhs.override { inherit ttyd; };
        in
        pkgs.writeShellApplication {
          name = "pi-rs-demo";
          runtimeInputs = [
            (mkPiRs system)
            vhs
          ];
          text = ''
            if [ -z "''${OPENROUTER_API_KEY:-}" ]; then
              echo "OPENROUTER_API_KEY is required to record the demo" >&2
              exit 1
            fi
            exec vhs ${./demo/pi-rs.tape} "$@"
          '';
        };
    in
    {
      checks = forAllSystems (system: {
        workspace-test = mkTest system;
        workspace-clippy = mkClippy system;
        launcher-smoke = mkLauncherSmoke system;
        model-catalog-update = mkModelCatalogUpdateTest system;
      });

      packages = forAllSystems (system: rec {
        pi-rs = mkPiRs system;
        update-model-catalog = mkModelCatalogUpdater system;
        default = pi-rs;
      });

      apps = forAllSystems (system: {
        demo = {
          type = "app";
          program = "${mkDemo system}/bin/pi-rs-demo";
        };
        update-model-catalog = {
          type = "app";
          program = "${mkModelCatalogUpdater system}/bin/update-model-catalog";
        };
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = mkPkgs system;
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              clippy
              rustfmt
              rust-analyzer
              stdenv.cc
              cargo-modules
              jq
              ripgrep
              fd
            ];
          };
        }
      );

      formatter = forAllSystems (system: (mkPkgs system).nixfmt-rfc-style);
    };
}
