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
              # The pi-rs-repl kernel shim (crates/pi-rs-repl/shim/kernel-shim.py)
              # is embedded into the crate via include_str!; the .py suffix must
              # survive the source snapshot. It is gate-allowlisted as
              # kernel_runtime (subprocess payload, like browser-export JS).
              || (lib.hasSuffix ".py" path)
              # The vendored prime-agent-runtime pyproject.toml (built as a
              # python package for the kernel env).
              || (lib.hasSuffix ".toml" path)
              # PLAN A.3: inert .ts fixtures (tests/model-catalog-update/*.generated.ts)
              # are parsed as data by the Rust update-model-catalog binary; keep them
              # in the source snapshot for the model-catalog-update check. They are
              # never executed (source-language-gate allowlists them as inert).
              || (lib.hasSuffix ".ts" path)
              # The autocomplete ui-parity scenario's deterministic fd
              # stand-in has no extension; keep it in the source tree so the
              # @-file-picker renders identically in the sandbox.
              || (lib.hasInfix "tests/ui-parity/fd-stub" path)
              # A.3: the gate script + its test tree are extensionless; keep
              # them in the source snapshot for the source-language-gate check.
              || (lib.hasInfix "scripts/source-language-gate" path)
              || (lib.hasInfix "tests/source-language-gate" path);
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
          # nixpkgs strip.sh bug: exit_code unbound when all strips succeed
          dontStrip = true;
          meta.mainProgram = "pi";
        };

      # Python environment for the pi-rs-repl kernel: IPython cell semantics,
      # dill snapshotting, nest-asyncio, and the vendored prime-agent-runtime
      # rlm package (crates/pi-rs-repl/vendor, copied from the pinned
      # ref/prime-agent oracle at c22549a). The vendored rlm is built as a
      # python package so the kernel imports it normally; its host_request is
      # redirected to the stdio bridge by the shim at boot.
      mkKernelPython =
        system:
        let
          c = mkCraneLib system;
          rlmPkg = c.pkgs.python3.pkgs.buildPythonPackage {
            pname = "prime-agent-runtime";
            version = "0.1.0";
            src = ./crates/pi-rs-repl/vendor/prime-agent-runtime;
            pyproject = true;
            nativeBuildInputs = [ c.pkgs.python3.pkgs.hatchling ];
            propagatedBuildInputs = [
              c.pkgs.python3.pkgs.ipykernel
              c.pkgs.python3.pkgs.nest-asyncio
              c.pkgs.python3.pkgs.tyro
            ];
            doCheck = false;
          };
        in
        c.pkgs.python3.withPackages (ps: [
          ps.ipython
          ps.dill
          ps.nest-asyncio
          rlmPkg
        ]);

      # The pi-rs-repl kernel bridge (crates/pi-rs-repl). The crate's
      # repl-smoke binary is the P1 gate consumer.
      mkRepl =
        system:
        let
          c = mkCraneLib system;
        in
        c.craneLib.buildPackage {
          inherit (c) src cargoArtifacts;
          pname = "pi-rs-repl";
          version = "0.1.0";
          nativeBuildInputs = c.commonEnv.nativeBuildInputs;
          cargoExtraArgs = "-p pi-rs-repl";
          doCheck = false;
          # nixpkgs strip.sh bug: exit_code unbound when all strips succeed
          dontStrip = true;
          meta.mainProgram = "repl-smoke";
        };

      # repl-smoke P1 gate app: the smoke binary wrapped with the kernel
      # Python env and the vendored rlm package on PYTHONPATH.
      mkReplSmokeApp =
        system:
        let
          c = mkCraneLib system;
        in
        c.pkgs.writeShellScriptBin "repl-smoke" ''
          export PI_RS_REPL_PYTHON=${mkKernelPython system}/bin/python3
          exec ${mkRepl system}/bin/repl-smoke "$@"
        '';

      # P1 gate as a flake check: runs the repl-smoke binary against the real
      # kernel env. Requires the Nix sandbox to allow process spawns (default
      # nix develop/run; the sandboxed check uses __noChroot or allow-builtin).
      mkReplSmokeCheck =
        system:
        let
          c = mkCraneLib system;
        in
        c.pkgs.runCommand "repl-smoke-check"
          {
            nativeBuildInputs = [ (mkReplSmokeApp system) ];
            __noChroot = true;
          }
          ''
            repl-smoke
            touch $out
          '';

      # The `.#prime` flake app (P3): the same `pi` binary with a declarative
      # composition that loads the Prime RLM Lua policy package
      # (prime/rlm.lua) through the public loader and dispatches to the
      # `prime-rlm` role it registers. The parity product never loads this
      # package; the composition mechanism is the generic `--role`/`--package`
      # path added to the launcher.
      mkPrimeApp =
        system:
        let
          c = mkCraneLib system;
        in
        c.pkgs.writeShellScriptBin "prime" ''
          export PI_RS_REPL_PYTHON=${mkKernelPython system}/bin/python3
          exec ${mkPiRs system}/bin/pi --role prime-rlm --package ${./prime/rlm.lua} "$@"
        '';

      # P3 gate check: the RLM loop runs end-to-end through the public loader
      # and drives a real kernel. The `prime_rlm_loop` test registers a
      # scripted API provider through the public `pi-rs-ai::registry` (no
      # dedicated test hook), loads `prime/rlm.lua` as an ordinary file-backed
      # package, dispatches the `prime-rlm` role, and asserts the loop reaches
      # a prose stop. It is skipped in the bare offline test env; this check
      # runs it under the Nix kernel python so `pi.repl` spawns a real IPython
      # child. Needs the sandbox to allow process spawns (__noChroot).
      mkPrimeRlmCheck =
        system:
        let
          c = mkCraneLib system;
        in
        c.craneLib.cargoTest {
          inherit (c) src cargoArtifacts;
          pname = "pi-rs-prime-rlm";
          version = "0.1.0";
          nativeBuildInputs = c.commonEnv.nativeBuildInputs;
          pnameSuffix = "-prime-rlm";
          cargoTestExtraArgs = "-p pi-rs-host --test prime_rlm_loop";
          # Provide the real kernel python env so the loop's kernel spawn works
          # during the check phase; process spawns need the unsandboxed runner.
          env = {
            PI_RS_REPL_PYTHON = "${mkKernelPython system}/bin/python3";
          };
          __noChroot = true;
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
            c.pkgs.nodejs # npm for the packages_transport git/npm transport tests
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

      # A.3 — Rust/Lua closure: fail-closed source-language gate over tracked
      # executable files and shebangs. Rejects any new first-party .ts/.py/.sh,
      # Python/shell shebang, or .js outside the browser-export allowlist that is
      # not in tests/source-language-gate/allowlist.json. The allowlist SHRINKS
      # as first-party source is ported to Rust/Lua or moved to a pinned oracle.
      # Intentional: this check executes bash/jq/find (mechanism), not any
      # repository-owned foreign-language program.
      mkSourceLanguageGate =
        system:
        let
          pkgs = mkPkgs system;
        in
        pkgs.runCommand "source-language-gate"
          {
            nativeBuildInputs = [
              pkgs.bash
              pkgs.coreutils
              pkgs.findutils
              pkgs.jq
            ];
          }
          ''
            bash ${self}/scripts/source-language-gate ${self}
            touch $out
          '';

      # Closed, offline Pi extension surface + translation/API-doc freshness gate.
      mkExtensionParity =
        system:
        let
          pkgs = mkPkgs system;
        in
        pkgs.runCommand "extension-parity"
          {
            nativeBuildInputs = [ pkgs.python3 ];
          }
          ''
            python3 ${self}/scripts/extension-inventory --check
            touch $out
          '';

      # PLAN A.3 — the model-catalog workflow is owned by Rust: the fixture-backed
      # normalization/rejection tests run the update-model-catalog binary against
      # the checked tests/model-catalog-update fixtures (Rust port of the deleted
      # scripts/test-model-catalog-update).
      mkModelCatalogUpdateTest =
        system:
        let
          c = mkCraneLib system;
        in
        c.craneLib.cargoTest {
          inherit (c) src cargoArtifacts;
          pname = "model-catalog-update-test";
          version = "0.1.0";
          nativeBuildInputs = c.commonEnv.nativeBuildInputs;
          cargoExtraArgs = "-p pi-rs-app --test model_catalog_update";
        };

      # Fail-closed first-party construction inventory: embedded source units,
      # declarations, Rust launch/composition seams, and named open risks must
      # all remain classified; negative controls pin rejection behavior.
      # PLAN A.3: the checker logic is the Rust test
      # crates/pi-rs-app/tests/construction_inventory_checker.rs, covered by the
      # workspace-test check; this check keeps the python oracle generator
      # (port_eligible, not yet ported) as its inventory-freshness gate.
      mkConstructionInventoryTest =
        system:
        let
          pkgs = mkPkgs system;
        in
        pkgs.runCommand "construction-inventory-test"
          {
            nativeBuildInputs = [ pkgs.python3 ];
          }
          ''
            python3 ${self}/scripts/construction-inventory --check
            touch $out
          '';

      # Offline, pinned-source capability inventory for the maintained external
      # extension dogfood suite. Includes idempotency and fail-closed controls.
      mkExternalExtensionInventoryTest =
        system:
        let
          pkgs = mkPkgs system;
        in
        pkgs.runCommand "external-extension-inventory-test"
          {
            nativeBuildInputs = [
              pkgs.bash
              pkgs.coreutils
              pkgs.gnugrep
              pkgs.python3
            ];
          }
          ''
            bash ${self}/scripts/test-external-extension-inventory
            touch $out
          '';

      # Offline maintained-extension fixture/provenance gate. The behavioral
      # source revision is checked into the contract; no sibling pi-flake checkout
      # is consulted by normal builds.
      # PLAN A.3: the contract check logic is the Rust test
      # crates/pi-rs-app/tests/dogfood_contract.rs, covered by the workspace-test
      # check; this check keeps the python oracle generator (port_eligible, not
      # yet ported) as its DOGFOOD_SUITE.md freshness gate.
      mkDogfoodFixtureTest =
        system:
        let
          pkgs = mkPkgs system;
        in
        pkgs.runCommand "dogfood-fixture-test"
          {
            nativeBuildInputs = [ pkgs.python3 ];
          }
          ''
            python3 ${self}/scripts/dogfood-oracle --check
            touch $out
          '';

      # Closed, offline source/public-surface audit against the pinned Pi
      # extraction. Reference regeneration is explicit and never reads an
      # ambient sibling checkout during normal checks.
      # PLAN A.1 — retained UI checkpoints compare continuously. Builds the
      # pi-rs package (compiled ui-diff) and runs the comparison loop.
      mkUiParity =
        system:
        let
          piRs = mkPiRs system;
        in
        piRs.overrideAttrs (
          final: prev: {
            nativeBuildInputs = prev.nativeBuildInputs or [ ] ++ [ (mkPkgs system).bash ];
            # Keep crane's default installPhase (installs the built `pi` and
            # `ui-diff` binaries to $out/bin via installFromCargoBuildLog) before
            # running the comparison loop; overrideAttrs would otherwise replace
            # it and leave $out/bin empty.
            installPhase = prev.installPhase + ''
              set -euo pipefail
              for name in basic-turn markdown-turn editor-turn autocomplete-turn shell-turn \
                provider-turn retry-turn bash-turn resume-turn session-turn tree-turn \
                compaction-turn tool-turn highlight-turn highlight-tool-turn selector-turn \
                login-turn model-turn thinking-turn settings-turn scoped-models-turn trust-turn \
                startup-changelog-turn reload-turn easter-eggs-turn extension-ui-turn; do
                $out/bin/ui-diff tests/ui-parity/$name.json tests/ui-parity/$name.pi.json \
                  >/dev/null || { echo "ui-parity: $name FAILED" >&2; exit 1; }
              done
            '';
            doCheck = false;
          }
        );

      mkFinalParityAudit =
        system:
        let
          pkgs = mkPkgs system;
        in
        pkgs.runCommand "final-parity-audit"
          {
            nativeBuildInputs = [ pkgs.python3 ];
          }
          ''
            python3 ${self}/scripts/final-parity-audit --check
            python3 ${self}/scripts/final-parity-audit --self-test
            touch $out
          '';

      # PLAN A.3 — the model-catalog updater is owned by Rust: the binary
      # (crates/pi-rs-app/src/bin/update-model-catalog.rs) discovers, downloads,
      # normalizes, and reviews the catalog; the deleted TypeScript counterpart
      # is gone. gnutar extracts the pinned npm tarball at runtime.
      mkModelCatalogUpdater =
        system:
        let
          c = mkCraneLib system;
        in
        c.craneLib.buildPackage {
          inherit (c) src cargoArtifacts;
          pname = "update-model-catalog";
          version = "0.1.0";
          nativeBuildInputs = c.commonEnv.nativeBuildInputs ++ [ c.pkgs.gnutar ];
          cargoExtraArgs = "-p pi-rs-app --bin update-model-catalog";
          doCheck = false;
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
        source-language-gate = mkSourceLanguageGate system;
        extension-parity = mkExtensionParity system;
        dogfood-fixtures = mkDogfoodFixtureTest system;
        ui-parity = mkUiParity system;
        final-parity-audit = mkFinalParityAudit system;
        repl-smoke = mkReplSmokeCheck system;
        prime-rlm = mkPrimeRlmCheck system;
      });

      packages = forAllSystems (system: rec {
        pi-rs = mkPiRs system;
        pi-rs-repl = mkRepl system;
        update-model-catalog = mkModelCatalogUpdater system;
        default = pi-rs;
      });

      apps = forAllSystems (system: {
        repl-smoke = {
          type = "app";
          program = "${mkReplSmokeApp system}/bin/repl-smoke";
        };
        prime = {
          type = "app";
          program = "${mkPrimeApp system}/bin/prime";
        };
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
