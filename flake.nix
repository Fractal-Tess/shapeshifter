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

      rustToolchain = pkgs: pkgs.rust-bin.stable.latest.default;

      mkRustPackage =
        pkgs:
        { pname, buildInputs ? [ ], nativeBuildInputs ? [ ], postFixup ? "" }:
        pkgs.rustPlatform.buildRustPackage {
          inherit pname;
          version = "0.2.0";
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
        shapeshifter-tui = mkRustPackage pkgs {
          pname = "shapeshifter-tui";
          buildInputs = tuiRuntimeDeps pkgs;
          postFixup = ''
            patchelf --set-rpath "${pkgs.lib.makeLibraryPath (tuiRuntimeDeps pkgs)}" $out/bin/shapeshifter-tui
          '';
        };

        shapeshifter-desktop = mkRustPackage pkgs {
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
