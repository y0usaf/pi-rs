{
  description = "pi-rs: Pi's packages in Rust, one crate each";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.pi = {
    url = "github:earendil-works/pi/f1b2e77f5b13b2a199b1052cb79c235451afe7d7";
    flake = false;
  };
  inputs.piExtensions = {
    url = "github:earendil-works/pi/4a080252f06caf63a525d7873c5d94d3c23887e0";
    flake = false;
  };
  inputs.piAi = {
    url = "github:earendil-works/pi/bdb151774ed6bef2b5676bbff4669f20a0c233fe";
    flake = false;
  };

  outputs =
    {
      self,
      nixpkgs,
      pi,
      piExtensions,
      piAi,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      pname = "pi-rs";
      inherit ((builtins.fromTOML (builtins.readFile ./crates/durable/Cargo.toml)).package) version;
      parityDirs = map toString [
        ./crates/ai/parity
        ./crates/durable/parity
      ];
      src = nixpkgs.lib.cleanSourceWith {
        src = nixpkgs.lib.cleanSource ./.;
        filter =
          path: _: !(builtins.any (dir: path == dir || nixpkgs.lib.hasPrefix (dir + "/") path) parityDirs);
      };
      build =
        pkgs: features:
        pkgs.rustPlatform.buildRustPackage {
          inherit pname version src;
          outputs = [
            "out"
            "examples"
          ];
          cargoLock.lockFile = ./Cargo.lock;
          doCheck = false;
          buildPhase = ''
            runHook preBuild
            cargo build --release --workspace --lib --bins --frozen --examples --features "${features}"
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            mkdir -p $out/lib $out/bin $examples/bin
            cp target/release/lib*.rlib $out/lib/
            find target/release/examples -maxdepth 1 -type f -executable -exec cp {} $examples/bin/ \;
            find target/release -maxdepth 1 -type f -executable -exec cp {} $out/bin/ \;
            runHook postInstall
          '';
          postInstall = ''
            mkdir -p $out/share/pi-rs
            cp -r ${extensionHost pkgs} $out/share/pi-rs/extension-host
          '';
          meta.mainProgram = "pi-rs";
        };
      chatManifest = (builtins.fromTOML (builtins.readFile ./examples/chat/Cargo.toml)).package;
      chatRuntime = pkgs: [
        pkgs.vulkan-loader
        pkgs.wayland
        pkgs.libxkbcommon
        pkgs.libx11
        pkgs.libxcb
      ];
      chat =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
          pname = chatManifest.name;
          inherit (chatManifest) version;
          inherit src;
          cargoRoot = "examples/chat";
          buildAndTestSubdir = "examples/chat";
          cargoLock = {
            lockFile = ./examples/chat/Cargo.lock;
            outputHashes = {
              "gpui-ce-0.2.2" = "sha256-AoSkSHlKqgeSwmT7QfdGuWwBvipy1PsqGwtlGsk7c3c=";
              "wgsl-rs-0.1.0" = "sha256-M+YP+viw8PADmxr92qg/6C3WweXBRvOTDuS1RKvzoBA=";
            };
          };
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [
            pkgs.fontconfig
            pkgs.freetype
          ]
          ++ chatRuntime pkgs;
          doCheck = false;
          preBuild = "export PI_CHAT_FONT=$out/share/pi-chat/InterVariable.ttf";
          postInstall = "install -Dm444 ${pkgs.inter}/share/fonts/truetype/InterVariable.ttf $PI_CHAT_FONT";
          postFixup = ''
            patchelf --add-rpath ${nixpkgs.lib.makeLibraryPath (chatRuntime pkgs)} $out/bin/pi-chat
          '';
          meta.mainProgram = "pi-chat";
        };
      npm =
        pkgs: name: version: hash:
        pkgs.fetchurl {
          url = "https://registry.npmjs.org/${name}/-/${name}-${version}.tgz";
          inherit hash;
        };
      oracleDeps = pkgs: {
        typebox = npm pkgs "typebox" "1.3.27" "sha256-WiiLzT1o9EVn3oSxheBNIF731bU2AoRrw9sw/dIZ3aM=";
        diff = npm pkgs "diff" "8.0.4" "sha256-71EyetWCQ89TMyYhn166pQKSgL0Pr8wR98IxVmOVk9Y=";
        partial-json =
          npm pkgs "partial-json" "0.1.7"
            "sha256-MOr+7W/cJfssok80w3eW2fmPC/XhVkUrmKLDVnYvYyI=";
      };
      unpackDeps =
        pkgs:
        nixpkgs.lib.concatStrings (
          nixpkgs.lib.mapAttrsToList (name: tgz: ''
            mkdir $out/node_modules/${name}
            tar -xzf ${tgz} --strip-components=1 -C $out/node_modules/${name}
          '') (oracleDeps pkgs)
        );
      oracle =
        pkgs:
        pkgs.runCommand "pi-ts-oracle" { } ''
          mkdir -p $out/packages $out/node_modules/@earendil-works $out/parity
          for p in ai chord durable; do
            mkdir -p $out/packages/$p
            cp -r ${pi}/packages/$p/src ${pi}/packages/$p/package.json $out/packages/$p
          done
          chmod -R u+w $out
          sed -i -E 's#\./dist/(.*)\.js"#./src/\1.ts"#' $out/packages/*/package.json
          ln -s ../../packages/ai $out/node_modules/@earendil-works/pi-ai
          ln -s ../../packages/chord $out/node_modules/@earendil-works/chord
          ln -s ../../packages/durable $out/node_modules/@earendil-works/pi-durable
          mkdir $out/packages/ai/src/providers/data
          for f in $(grep -ho 'data/[a-z0-9-]*\.json' $out/packages/ai/src/providers/*.models.ts | sort -u) data/.manifest.json; do
            echo '{}' > $out/packages/ai/src/providers/$f
          done
          ${unpackDeps pkgs}
          cp ${./crates/durable/parity}/{driver.ts,register.mjs,loader.mjs} $out/parity/
        '';
      oracleAi =
        pkgs:
        pkgs.runCommand "pi-ts-oracle-ai" { } ''
          mkdir -p $out/packages $out/node_modules/@earendil-works $out/parity-ai
          for p in ai telemetry; do
            mkdir -p $out/packages/$p
            cp -r ${piAi}/packages/$p/src ${piAi}/packages/$p/package.json $out/packages/$p
          done
          chmod -R u+w $out
          sed -i -E 's#\./dist/(.*)\.js"#./src/\1.ts"#' $out/packages/*/package.json
          ln -s ../../packages/ai $out/node_modules/@earendil-works/pi-ai
          ln -s ../../packages/telemetry $out/node_modules/@earendil-works/pi-telemetry
          mkdir $out/packages/ai/src/providers/data
          for f in $(grep -ho 'data/[a-z0-9-]*\.json' $out/packages/ai/src/providers/*.models.ts | sort -u) data/.manifest.json; do
            echo '{}' > $out/packages/ai/src/providers/$f
          done
          ${unpackDeps pkgs}
          cp ${./crates/ai/parity/driver-ai.ts} $out/parity-ai/driver-ai.ts
          cp ${./crates/ai/parity/register.mjs} $out/parity-ai/register.mjs
          cp ${./crates/ai/parity/loader.mjs} $out/parity-ai/loader.mjs
          cp -r ${./crates/ai/parity/stubs} $out/parity-ai/stubs
        '';
      upstreamPi = pkgs: pkgs.callPackage "${piExtensions}/nix/package.nix" { source = piExtensions; };
      upstreamPackage = pkgs: "${upstreamPi pkgs}/lib/pi/node_modules/@earendil-works/pi-coding-agent";
      extensionsParityFixtures =
        pkgs:
        let
          examples = "${piExtensions}/packages/coding-agent/examples/extensions";
          locked = builtins.filter (name: builtins.pathExists "${examples}/${name}/package-lock.json") (
            builtins.attrNames (pkgs.lib.filterAttrs (_: kind: kind == "directory") (builtins.readDir examples))
          );
          modules =
            name:
            pkgs.importNpmLock.buildNodeModules {
              npmRoot = "${examples}/${name}";
              nodejs = pkgs.nodejs_24;
            };
        in
        pkgs.runCommand "pi-extensions-parity-fixtures" { } ''
          cp -r ${./crates/coding-agent/parity/extensions} $out
          chmod u+w $out
          cp -r ${examples} $out/upstream
          chmod -R u+w $out/upstream
          ${pkgs.lib.concatMapStrings (name: ''
            ln -s ${modules name}/node_modules $out/upstream/${name}/node_modules
          '') locked}
        '';
      extensionHost =
        pkgs:
        let
          modules = "${upstreamPi pkgs}/lib/pi/node_modules";
          host = ./crates/extensions/host;
        in
        pkgs.runCommand "pi-extension-host" { } ''
          mkdir -p $out/node_modules/@earendil-works
          for module in ${modules}/*; do
            [ "$(basename "$module")" = @earendil-works ] || ln -s "$module" $out/node_modules/
          done
          for module in ${modules}/@earendil-works/*; do
            case "$(basename "$module")" in
              pi-coding-agent | pi-ai) cp -r "$module" $out/node_modules/@earendil-works/ ;;
              *) ln -s "$module" $out/node_modules/@earendil-works/ ;;
            esac
          done
          chmod -R u+w $out/node_modules/@earendil-works
          agent=$out/node_modules/@earendil-works/pi-coding-agent
          mv $agent/dist/index.js $agent/dist/upstream.js
          cp ${host}/index.mjs $agent/dist/index.js
          cp ${host}/exec.mjs $agent/dist/core/exec.js
          ai=$out/node_modules/@earendil-works/pi-ai
          mv $ai/dist/providers/all.js $ai/dist/providers/all-upstream.js
          cp ${host}/providers-all.mjs $ai/dist/providers/all.js
          cp ${host}/host.mjs $out/
          ln -s ${pkgs.nodejs_24}/bin/node $out/node
        '';
    in
    {
      packages = forAll (pkgs: {
        default = build pkgs "pi-durable/providers";
        storage-only = build pkgs "";
        extension-host = extensionHost pkgs;
        chat = chat pkgs;
      });

      apps = forAll (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/pi-rs";
        };
        tui = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default.examples}/bin/tui_editor";
        };
      });

      checks = forAll (
        pkgs:
        let
          system = pkgs.stdenv.hostPlatform.system;
          examples = self.packages.${system}.default.examples;
        in
        {
          build = self.packages.${system}.default;
          storage-only = self.packages.${system}.storage-only;
          chat = self.packages.${system}.chat;
          clippy = self.packages.${system}.default.overrideAttrs (old: {
            pname = "${pname}-clippy";
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.clippy ];
            buildPhase = "cargo clippy --workspace --all-targets --frozen --features pi-durable/providers -- -D warnings";
            installPhase = "mkdir -p $out $examples";
          });
          tests = self.packages.${system}.default.overrideAttrs (old: {
            pname = "${pname}-tests";
            buildPhase = "cargo test --release --workspace --frozen --features pi-durable/providers";
            installPhase = "mkdir -p $out $examples";
          });
          oauth-flow = pkgs.runCommand "${pname}-oauth-flow" { } ''
            ${examples}/bin/oauth_flow
            touch $out
          '';
          provider-e2e = pkgs.runCommand "${pname}-provider-e2e" { } ''
            ${examples}/bin/provider_e2e
            touch $out
          '';
          openai-responses-e2e = pkgs.runCommand "${pname}-openai-responses-e2e" { } ''
            ${examples}/bin/openai_responses_e2e
            touch $out
          '';
          openai-completions-e2e = pkgs.runCommand "${pname}-openai-completions-e2e" { } ''
            ${examples}/bin/openai_completions_e2e
            touch $out
          '';
          modes-e2e = pkgs.runCommand "${pname}-modes-e2e" { } ''
            PI_RS_BIN=${self.packages.${system}.default}/bin/pi-rs ${examples}/bin/modes_e2e
            touch $out
          '';
          mount-e2e = pkgs.runCommand "${pname}-mount-e2e" { } ''
            PI_RS_BIN=${self.packages.${system}.default}/bin/pi-rs ${examples}/bin/mount_e2e
            touch $out
          '';
          interactive-e2e = pkgs.runCommand "${pname}-interactive-e2e" { } ''
            PI_RS_BIN=${self.packages.${system}.default}/bin/pi-rs ${examples}/bin/interactive_e2e
            touch $out
          '';
          session-e2e = pkgs.runCommand "${pname}-session-e2e" { } ''
            PI_RS_BIN=${self.packages.${system}.default}/bin/pi-rs ${examples}/bin/session_e2e
            touch $out
          '';
          login-e2e = pkgs.runCommand "${pname}-login-e2e" { } ''
            ${examples}/bin/login_e2e
            touch $out
          '';
          auth-e2e = pkgs.runCommand "${pname}-auth-e2e" { } ''
            PI_RS_BIN=${self.packages.${system}.default}/bin/pi-rs ${examples}/bin/auth_e2e
            touch $out
          '';
          mcp-e2e = pkgs.runCommand "${pname}-mcp-e2e" { } ''
            PI_RS_BIN=${self.packages.${system}.default}/bin/pi-rs ${examples}/bin/mcp_e2e
            touch $out
          '';
          durable-sleep-e2e = pkgs.runCommand "${pname}-durable-sleep-e2e" { } ''
            ${examples}/bin/durable_sleep_e2e
            touch $out
          '';
          extensions-e2e = pkgs.runCommand "${pname}-extensions-e2e" { } ''
            PI_RS_BIN=${self.packages.${system}.default}/bin/pi-rs \
              PI_EXTENSION_HOST=${extensionHost pkgs} \
              ${examples}/bin/extensions_e2e
            touch $out
          '';
          extensions-parity =
            pkgs.runCommand "${pname}-extensions-parity"
              {
                passthru = {
                  upstream = upstreamPi pkgs;
                  fixtures = extensionsParityFixtures pkgs;
                };
              }
              ''
                PI_UPSTREAM_BIN=${upstreamPi pkgs}/bin/pi \
                  PI_UPSTREAM_PACKAGE_DIR=${upstreamPackage pkgs} \
                  PI_RS_BIN=${self.packages.${system}.default}/bin/pi-rs \
                  PI_RS_PACKAGE_DIR=${self.packages.${system}.default}/bin \
                  PI_EXTENSION_HOST=${extensionHost pkgs} \
                  PI_PARITY_FIXTURES=${extensionsParityFixtures pkgs} \
                  ${examples}/bin/extensions_parity
                touch $out
              '';
          extensions-parity-baseline = pkgs.runCommand "${pname}-extensions-parity-baseline" { } ''
            PI_UPSTREAM_BIN=${upstreamPi pkgs}/bin/pi \
              PI_UPSTREAM_PACKAGE_DIR=${upstreamPackage pkgs} \
              PI_RS_BIN=${upstreamPi pkgs}/bin/pi \
              PI_RS_PACKAGE_DIR=${upstreamPackage pkgs} \
              PI_PARITY_FIXTURES=${extensionsParityFixtures pkgs} \
              ${examples}/bin/extensions_parity
            touch $out
          '';
          tui-parity = pkgs.runCommand "${pname}-tui-parity" { } ''
            PI_UPSTREAM_BIN=${upstreamPi pkgs}/bin/pi \
              PI_RS_BIN=${self.packages.${system}.default}/bin/pi-rs \
              PI_EXTENSION_HOST=${extensionHost pkgs} \
              PI_PARITY_FIXTURES=${extensionsParityFixtures pkgs} \
              ${examples}/bin/tui_parity
            touch $out
          '';
          tui-parity-baseline = pkgs.runCommand "${pname}-tui-parity-baseline" { } ''
            PI_UPSTREAM_BIN=${upstreamPi pkgs}/bin/pi \
              PI_RS_BIN=${upstreamPi pkgs}/bin/pi \
              PI_PARITY_FIXTURES=${extensionsParityFixtures pkgs} \
              ${examples}/bin/tui_parity
            touch $out
          '';
          codemode-e2e = pkgs.runCommand "${pname}-codemode-e2e" { } ''
            PI_RS_BIN=${self.packages.${system}.default}/bin/pi-rs ${examples}/bin/codemode_e2e
            touch $out
          '';
          parity =
            pkgs.runCommand "${pname}-parity"
              {
                nativeBuildInputs = [
                  pkgs.nodejs_24
                  pkgs.gnugrep
                ];
                PARITY_TS = "${oracle pkgs}/parity";
                PARITY_RS = "${examples}/bin/parity";
              }
              ''
                bash ${./crates/durable/parity}/run.sh
                touch $out
              '';
          parity-ai =
            pkgs.runCommand "${pname}-parity-ai"
              {
                nativeBuildInputs = [
                  pkgs.nodejs_24
                  pkgs.diffutils
                ];
                PARITY_AI_TS = "${oracleAi pkgs}";
                PARITY_AI_RS = "${examples}/bin/parity_ai";
                passthru.oracle = oracleAi pkgs;
              }
              ''
                bash ${./crates/ai/parity}/run.sh
                touch $out
              '';
          fmt = pkgs.runCommand "${pname}-fmt" {
            nativeBuildInputs = [
              pkgs.cargo
              pkgs.rustfmt
            ];
          } "cd ${src} && cargo fmt --check && touch $out";
        }
      );

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.clippy
            pkgs.rustfmt
            pkgs.nodejs
          ];
        };
        chat = pkgs.mkShell {
          inputsFrom = [ (chat pkgs) ];
          packages = [
            pkgs.clippy
            pkgs.rustfmt
          ];
          LD_LIBRARY_PATH = nixpkgs.lib.makeLibraryPath (chatRuntime pkgs);
          PI_CHAT_FONT = "${pkgs.inter}/share/fonts/truetype/InterVariable.ttf";
        };
      });

      formatter = forAll (pkgs: pkgs.nixfmt);
    };
}
