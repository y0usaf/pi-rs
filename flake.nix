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

      # Raw zero-policy `pi` binary (crates/pi-rs-app). The default
      # distribution is the same artifact until its manifest lands in 3.6.
      mkPiCore =
        system:
        let
          c = mkCraneLib system;
        in
        c.craneLib.buildPackage {
          inherit (c) src cargoArtifacts;
          pname = "pi-core";
          version = "0.1.0";
          nativeBuildInputs = c.commonEnv.nativeBuildInputs ++ [
            c.pkgs.ripgrep
            c.pkgs.fd
          ];
          cargoExtraArgs = "-p pi-rs-app";
          doCheck = false;
          meta.mainProgram = "pi";
        };

      mkPiRs = system: mkPiCore system;

      # Raw-core ablation: intentional absence is useful guidance, not a
      # failed product launch.
      mkCoreNoPackageCheck =
        system:
        let
          pkgs = mkPkgs system;
          piCore = mkPiCore system;
        in
        pkgs.runCommand "pi-core-no-package"
          {
            nativeBuildInputs = [ piCore ];
          }
          ''
            export HOME=$TMPDIR
            unset PI_PACKAGE_MANIFEST
            pi > guidance.txt 2> error.txt
            test ! -s error.txt
            grep -q 'no application package selected' guidance.txt
            grep -q -- '--package FILE or --manifest FILE' guidance.txt
            touch $out
          '';

      # Raw-core capability: an ordinary file package reaches the canonical
      # package transaction and application-root dispatch.
      mkCoreFileApplicationCheck =
        system:
        let
          pkgs = mkPkgs system;
          piCore = mkPiCore system;
        in
        pkgs.runCommand "pi-core-file-application"
          {
            nativeBuildInputs = [
              piCore
              pkgs.jq
            ];
          }
          ''
            export HOME=$TMPDIR
            unset PI_PACKAGE_MANIFEST

            pi --help > help.txt
            grep -q 'generic Lua application launcher' help.txt
            grep -q -- '--manifest FILE' help.txt

            cat > application.lua <<'LUA'
            local roots = (...).roots.v1
            roots.register({
              kind = "application",
              id = "core-file-check",
              dispatch = function(snapshot)
                roots.action("launched", {
                  argument = snapshot.event.arguments[1],
                  root = snapshot.context.root,
                })
                roots.action("shutdown", { reason = "complete" })
              end,
            })
            LUA

            pi --root "$TMPDIR" --package application.lua -- accepted > result.json
            jq -e '
              .version == 1 and
              .actions[0].kind == "launched" and
              .actions[0].payload.argument == "accepted" and
              .actions[1].kind == "shutdown" and
              (.source | endswith("/application.lua"))
            ' result.json >/dev/null

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
        core-no-package = mkCoreNoPackageCheck system;
        core-file-application = mkCoreFileApplicationCheck system;
        model-catalog-update = mkModelCatalogUpdateTest system;
      });

      packages = forAllSystems (system: rec {
        pi-core = mkPiCore system;
        pi-rs = mkPiRs system;
        update-model-catalog = mkModelCatalogUpdater system;
        default = pi-rs;
      });

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${mkPiRs system}/bin/pi";
        };
        pi-rs = {
          type = "app";
          program = "${mkPiRs system}/bin/pi";
        };
        pi-core = {
          type = "app";
          program = "${mkPiCore system}/bin/pi";
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
