{
  description = "pi-rs — pi ported to Rust";

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

      # Doctrine 06 — bare core boots: the substrate with zero packs,
      # zero config, and zero credentials still runs and does something
      # minimal but real. Exercises the WS2.6 entry points headlessly.
      mkBareBoot =
        system:
        let
          pkgs = mkPkgs system;
          piRs = mkPiRs system;
        in
        pkgs.runCommand "bare-boot"
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

      # Clippy with warnings denied — the code standard (no unwrap/expect/
      # panic in library crates) is enforced here, not aspirational.
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
          cargoClippyExtraArgs = "--workspace --all-targets -- --deny warnings";
        };

      # ARCHITECTURE.md is generated by scripts/gen-arch.sh; this check
      # regenerates it in the sandbox and fails if the committed copy is
      # stale.
      mkArchFresh =
        system:
        let
          pkgs = mkPkgs system;
        in
        pkgs.stdenv.mkDerivation {
          name = "arch-fresh";
          src = self;
          nativeBuildInputs = [
            pkgs.rustPlatform.cargoSetupHook
            pkgs.cargo
            pkgs.rustc
            pkgs.jq
            pkgs.cargo-modules
          ];
          cargoDeps = pkgs.rustPlatform.importCargoLock { lockFile = ./Cargo.lock; };
          buildPhase = ''
            export HOME=$TMPDIR
            cp ARCHITECTURE.md $TMPDIR/committed.md
            bash scripts/gen-arch.sh
            diff -u $TMPDIR/committed.md ARCHITECTURE.md || {
              echo 'ARCHITECTURE.md is stale — run scripts/gen-arch.sh and commit the result.' >&2
              exit 1
            }
            touch $out
          '';
          dontInstall = true;
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
    in
    {
      checks = forAllSystems (system: {
        workspace-test = mkTest system;
        workspace-clippy = mkClippy system;
        arch-fresh = mkArchFresh system;
        bare-boot = mkBareBoot system;
        model-catalog-update = mkModelCatalogUpdateTest system;
      });

      packages = forAllSystems (system: rec {
        pi-rs = mkPiRs system;
        update-model-catalog = mkModelCatalogUpdater system;
        default = pi-rs;
      });

      apps = forAllSystems (system: {
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
