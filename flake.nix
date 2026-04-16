{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    systems.url = "github:nix-systems/default";
  };
  outputs =
    {
      self,
      systems,
      nixpkgs,
      ...
    }@inputs:
    let
      version = "0.2.1";
      repo = "Fractal-Tess/shapeshifter";

      eachSystem =
        f:
        nixpkgs.lib.genAttrs (import systems) (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ inputs.rust-overlay.overlays.default ];
            }
          )
        );

      commonRuntimeDeps =
        pkgs: with pkgs; [
          fontconfig
          libGL
          libglvnd
          libx11
          libxcb
          libxkbcommon
          libxi
          wayland
          xorg.libXcursor
          xorg.libXext
          xorg.libXfixes
          xorg.libXinerama
          xorg.libXrandr
          xorg.libXrender
          xorg.libXxf86vm
        ];

      tuiRuntimeDeps =
        pkgs: with pkgs; [
          libxcb
          xorg.libXau
          xorg.libXdmcp
        ];

      runtimeLibraryPath =
        pkgs:
        "/run/opengl-driver/lib:${pkgs.lib.makeLibraryPath (commonRuntimeDeps pkgs)}";

      # Prebuilt binary packages from GitHub releases
      mkPrebuilt =
        pkgs:
        { pname, assetName, sha256, runtimeDeps ? [ ], wrapWithLibs ? false }:
        let
          src = pkgs.fetchurl {
            url = "https://github.com/${repo}/releases/download/v${version}/${assetName}";
            inherit sha256;
          };
        in
        pkgs.stdenv.mkDerivation {
          inherit pname version;
          dontUnpack = true;
          nativeBuildInputs = pkgs.lib.optionals wrapWithLibs [ pkgs.makeWrapper pkgs.autoPatchelfHook ];
          buildInputs = runtimeDeps;
          installPhase = ''
            mkdir -p $out/bin
            cp ${src} $out/bin/${pname}
            chmod +x $out/bin/${pname}
          '';
          postFixup =
            if wrapWithLibs then ''
              wrapProgram $out/bin/${pname} \
                --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath runtimeDeps}"
            '' else "";
          meta = {
            description = "ChatGPT / Codex account manager";
            license = pkgs.lib.licenses.mit;
            mainProgram = pname;
            platforms = [ "x86_64-linux" ];
          };
        };

      # Build from source (fallback / dev)
      mkFromSource =
        pkgs:
        { pname, buildInputs ? [ ], nativeBuildInputs ? [ ], postFixup ? "" }:
        pkgs.rustPlatform.buildRustPackage {
          inherit pname version;
          src = ./.;
          buildAndTestSubdir = if pname == "shapeshifter-tui" then "apps/shapeshifter-tui" else "apps/shapeshifter-desktop";
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.pkg-config ] ++ nativeBuildInputs;
          buildInputs = buildInputs;
          postFixup = postFixup;
          meta = {
            description = "ChatGPT / Codex account manager";
            license = pkgs.lib.licenses.mit;
            mainProgram = pname;
          };
        };
    in
    {
      packages = eachSystem (pkgs: {
        shapeshifter-tui = mkPrebuilt pkgs {
          pname = "shapeshifter-tui";
          assetName = "shapeshifter-linux-x86_64-tui";
          sha256 = "sha256-e9fa0m8PMI+n8Jkof+nKNQchgl0ot4iN4KTOyfNIyOg=";
          runtimeDeps = tuiRuntimeDeps pkgs;
        };

        shapeshifter-desktop = mkPrebuilt pkgs {
          pname = "shapeshifter-desktop";
          assetName = "shapeshifter-linux-x86_64";
          sha256 = "sha256-YpTBBWE8azgbtabZuU8wQZdqXWmkaPzlkvGOFXgkxdA=";
          runtimeDeps = commonRuntimeDeps pkgs;
          wrapWithLibs = true;
        };

        # Build from source variants
        shapeshifter-tui-dev = mkFromSource pkgs {
          pname = "shapeshifter-tui";
          buildInputs = tuiRuntimeDeps pkgs;
          postFixup = ''
            patchelf --set-rpath "${pkgs.lib.makeLibraryPath (tuiRuntimeDeps pkgs)}" $out/bin/shapeshifter-tui
          '';
        };

        shapeshifter-desktop-dev = mkFromSource pkgs {
          pname = "shapeshifter-desktop";
          buildInputs = commonRuntimeDeps pkgs;
          nativeBuildInputs = [ pkgs.makeWrapper ];
          postFixup = ''
            wrapProgram $out/bin/shapeshifter-desktop \
              --prefix LD_LIBRARY_PATH : "${runtimeLibraryPath pkgs}"
          '';
        };

        default = self.packages.${pkgs.system}.shapeshifter-tui;
      });

      overlays.default = final: prev: {
        shapeshifter-tui = self.packages.${final.system}.shapeshifter-tui;
        shapeshifter-desktop = self.packages.${final.system}.shapeshifter-desktop;
      };

      devShells = eachSystem (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            (rust-bin.stable.latest.default.override {
              extensions = [
                "rust-analyzer"
                "clippy"
                "rustfmt"
                "rust-src"
                "rust-docs"
              ];
              targets = [ "x86_64-unknown-linux-musl" ];
            })
          ] ++ commonRuntimeDeps pkgs;
          RUST_SRC_PATH = "${pkgs.rust-bin.stable.latest.rust-src}/lib/rustlib/src/rust/library";
          LD_LIBRARY_PATH = runtimeLibraryPath pkgs;
          shellHook = ''
            export XDG_DATA_DIRS="/run/opengl-driver/share:''${XDG_DATA_DIRS:-}"
          '';
        };
      });
    };
}
