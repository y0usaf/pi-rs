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
              || (lib.hasSuffix ".pem" path)
              || (lib.hasSuffix ".sse" path)
              || (lib.hasSuffix ".txt" path)
              || (lib.hasSuffix ".png" path);
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

      # Build the A.3 Rust tools binary (source-language gate + every migrated
      # generator/inventory owner). `nix flake check` needs no repo-owned
      # Node/Bun/Python/shell runtime.
      mkPiRsTools =
        system:
        let
          c = mkCraneLib system;
        in
        c.craneLib.buildPackage {
          inherit (c) src cargoArtifacts;
          pname = "pi-rs-tools";
          version = "0.1.0";
          nativeBuildInputs = c.commonEnv.nativeBuildInputs;
          cargoExtraArgs = "-p pi-rs-tools";
          doCheck = false;
          meta.mainProgram = "pi-rs-tools";
        };

      # Closed, offline Pi extension surface + translation/API-doc freshness
      # gate. Rust owner (`pi-rs-tools extension-inventory {check,selftest}`);
      # no repo-owned Python runtime.
      mkExtensionParity =
        system:
        let
          c = mkCraneLib system;
          tools = mkPiRsTools system;
        in
        c.pkgs.runCommand "extension-parity"
          {
            nativeBuildInputs = [ tools ];
          }
          ''
            pi-rs-tools extension-inventory check --root ${self}
            pi-rs-tools extension-inventory selftest --root ${self}
            touch $out
          '';

      # A.3 source-language gate: the workspace builds a Rust gate binary that
      # rejects new first-party .ts/.py/.sh/shebang files and .js outside the
      # browser-export allowlist. The check needs no Node/Bun/Python runtime.
      mkSourceLanguageGate =
        system:
        let
          c = mkCraneLib system;
          tools = c.craneLib.buildPackage {
            inherit (c) src cargoArtifacts;
            pname = "pi-rs-tools";
            version = "0.1.0";
            nativeBuildInputs = c.commonEnv.nativeBuildInputs ++ [ c.pkgs.git ];
            cargoExtraArgs = "-p pi-rs-tools";
            doCheck = false;
            meta.mainProgram = "pi-rs-tools";
          };
        in
        c.pkgs.runCommand "source-language-gate"
          {
            nativeBuildInputs = [
              tools
              c.pkgs.git
            ];
          }
          ''
            # The flake source is a git-less store path, so the gate's file
            # enumeration falls back to a recursive walk of the store tree
            # (equivalent for gating since the source is already clean).
            pi-rs-tools gate check --root ${self} > out.txt || {
              echo "A.3 source-language gate FAILED:" >&2
              cat out.txt >&2
              exit 1
            }
            cat out.txt
            touch $out
          '';

      # Offline, fixture-backed normalization and rejection tests for the
      # reviewed model-catalog update path. Driven by the Rust model-catalog
      # owner (`pi-rs-tools model-catalog selftest`); no Node/Bun runtime.
      mkModelCatalogUpdateTest =
        system:
        let
          c = mkCraneLib system;
          tools = c.craneLib.buildPackage {
            inherit (c) src cargoArtifacts;
            pname = "pi-rs-tools";
            version = "0.1.0";
            nativeBuildInputs = c.commonEnv.nativeBuildInputs;
            cargoExtraArgs = "-p pi-rs-tools";
            doCheck = false;
            meta.mainProgram = "pi-rs-tools";
          };
        in
        c.pkgs.runCommand "model-catalog-update-test"
          {
            nativeBuildInputs = [ tools ];
          }
          ''
            pi-rs-tools model-catalog selftest --root ${self}
            touch $out
          '';

      # Fail-closed first-party construction inventory: embedded source units,
      # declarations, Rust launch/composition seams, and named open risks must
      # all remain classified; negative controls pin rejection behavior. Rust
      # owner (`pi-rs-tools construction-inventory {check,selftest}`); no
      # repo-owned Python runtime.
      mkConstructionInventoryTest =
        system:
        let
          c = mkCraneLib system;
          tools = c.craneLib.buildPackage {
            inherit (c) src cargoArtifacts;
            pname = "pi-rs-tools";
            version = "0.1.0";
            nativeBuildInputs = c.commonEnv.nativeBuildInputs;
            cargoExtraArgs = "-p pi-rs-tools";
            doCheck = false;
            meta.mainProgram = "pi-rs-tools";
          };
        in
        c.pkgs.runCommand "construction-inventory-test"
          {
            nativeBuildInputs = [ tools ];
          }
          ''
            pi-rs-tools construction-inventory --check --root ${self}
            pi-rs-tools construction-inventory selftest --root ${self}
            touch $out
          '';

      # Offline, pinned-source capability inventory for the maintained external
      # extension dogfood suite. Rust owner (`pi-rs-tools
      # external-extension-inventory {check,selftest}`); includes idempotency and
      # fail-closed controls while needing no repo-owned Python/bash.
      mkExternalExtensionInventoryTest =
        system:
        let
          c = mkCraneLib system;
          tools = mkPiRsTools system;
        in
        c.pkgs.runCommand "external-extension-inventory-test"
          {
            nativeBuildInputs = [ tools ];
          }
          ''
            pi-rs-tools external-extension-inventory check --root ${self}
            pi-rs-tools external-extension-inventory selftest --root ${self}
            touch $out
          '';

      # A.2 hash-lock oracle input: the pinned `pi-flake` revision that the
      # committed external-extension fixtures are extracted from. This is a
      # fixed-output `fetchgit`, so Nix fetches it only when a derivation
      # depending on it is built (opt-in oracle regeneration / /
      # re-verification). Normal `nix flake check` never builds it and stays
      # fully offline against the committed fixtures + provenance.json.
      mkPiFlakeOracle =
        pkgs:
        pkgs.fetchgit {
          url = "https://github.com/y0usaf/pi-flake";
          # Pin to the exact revision recorded in
          # tests/external-extension-inventory/provenance.json.
          rev = "94694da7321ce74aa7b82c13db7e60e28c0caba6";
          sha256 = "sha256-PD3E5KPP50AuAddI0mFgdZqjKN6BTR+YAv3l7Y+Nv9A=";
        };

      # Opt-in: make the hash-locked pi-flake oracle available and expose the
      # pinned revision, so provenance can be re-verified/regenerated against a
      # byte-identical pinned tree without altering normal offline checks.
      mkRefreshExternalExtensionFixtures =
        system:
        let
          pkgs = mkPkgs system;
          oracle = mkPiFlakeOracle pkgs;
        in
        pkgs.writeShellApplication {
          name = "refresh-external-extension-fixtures";
          runtimeInputs = [ pkgs.git ];
          text = ''
            echo "hash-locked pi-flake oracle (A.2):"
            echo "  oracle = ${oracle}"
            echo "  revision = 94694da7321ce74aa7b82c13db7e60e28c0caba6"
            echo "  extensions tree = c4a04dfe88314b5e48ebb200ccfd546645c3af9e"
            echo "Regenerate tests/external-extension-inventory/provenance.json from"
            echo "this pinned tree; the committed fixtures are not touched by default."
          '';
        };

      # Offline maintained-extension fixture/provenance gate. Rust owner
      # (`pi-rs-tools dogfood-oracle {check,selftest}`); the behavioral source
      # revision is checked into the contract and no sibling pi-flake checkout is
      # consulted by normal builds.
      mkDogfoodFixtureTest =
        system:
        let
          c = mkCraneLib system;
          tools = mkPiRsTools system;
        in
        c.pkgs.runCommand "dogfood-fixture-test"
          {
            nativeBuildInputs = [ tools ];
          }
          ''
            pi-rs-tools dogfood-oracle check --root ${self}
            pi-rs-tools dogfood-oracle selftest --root ${self}
            touch $out
          '';

      # Closed, offline source/public-surface audit against the pinned Pi
      # extraction. Rust owner (`pi-rs-tools final-parity-audit {check,selftest}`).
      # Reference regeneration is explicit and never reads an ambient sibling
      # checkout during normal checks.
      mkFinalParityAudit =
        system:
        let
          c = mkCraneLib system;
          tools = mkPiRsTools system;
        in
        c.pkgs.runCommand "final-parity-audit"
          {
            nativeBuildInputs = [ tools ];
          }
          ''
            pi-rs-tools final-parity-audit check --root ${self}
            pi-rs-tools final-parity-audit selftest --root ${self}
            touch $out
          '';

      mkModelCatalogUpdater =
        system:
        let
          c = mkCraneLib system;
          tools = c.craneLib.buildPackage {
            inherit (c) src cargoArtifacts;
            pname = "pi-rs-tools";
            version = "0.1.0";
            nativeBuildInputs = c.commonEnv.nativeBuildInputs ++ [ c.pkgs.git ];
            cargoExtraArgs = "-p pi-rs-tools";
            doCheck = false;
            meta.mainProgram = "pi-rs-tools";
          };
        in
        c.pkgs.writeShellApplication {
          name = "update-model-catalog";
          runtimeInputs = [
            tools
            c.pkgs.git
          ];
          text = ''
            exec pi-rs-tools model-catalog update "$@"
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
        arch-fresh = mkArchFresh system;
        bare-boot = mkBareBoot system;
        model-catalog-update = mkModelCatalogUpdateTest system;
        construction-inventory = mkConstructionInventoryTest system;
        external-extension-inventory = mkExternalExtensionInventoryTest system;
        extension-parity = mkExtensionParity system;
        dogfood-fixtures = mkDogfoodFixtureTest system;
        final-parity-audit = mkFinalParityAudit system;
        source-language-gate = mkSourceLanguageGate system;
      });

      packages = forAllSystems (system: rec {
        pi-rs = mkPiRs system;
        update-model-catalog = mkModelCatalogUpdater system;
        # A.2 hash-locked oracle input for the external-extension fixtures.
        pi-flake-oracle = mkPiFlakeOracle (mkPkgs system);
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
        refresh-external-extension-fixtures = {
          type = "app";
          program = "${mkRefreshExternalExtensionFixtures system}/bin/refresh-external-extension-fixtures";
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
