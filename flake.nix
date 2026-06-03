{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            pkgs.python3

            # Rust toolchain (cargo, rustc, clippy, rustfmt, rust-src) from the
            # rust-overlay, plus rust-analyzer for editor support.
            (pkgs.rust-bin.stable.latest.default.override {
              extensions = [ "rust-src" "rust-analyzer" ];
            })

            # Build Python-Api
            pkgs.maturin

            # Testing
            pkgs.magic-vlsi
            pkgs.klayout
            pkgs.netgen
          ];

          shellHook = ''
            # Python venv (inherits nix packages)
            if [ ! -d .venv ]; then
              python3 -m venv .venv --system-site-packages
              .venv/bin/pip install --quiet ciel gdstk
            fi
            source .venv/bin/activate

            export PDK_ROOT="''${PDK_ROOT:-$HOME/.ciel}"
            export SKY130_PDK_VERSION="''${SKY130_PDK_VERSION:-}"
            export GF180MCU_PDK_VERSION="''${GF180MCU_PDK_VERSION:-}"
            export IHP_SG13G2_PDK_VERSION="''${IHP_SG13G2_PDK_VERSION:-}"

            # Ciel PDKs
            resolve_latest_pdk_version() {
              local family="$1"
              python3 -m ciel ls-remote --pdk-family "$family" 2>/dev/null | head -n 1
            }

            pdk_enabled() {
              local family="$1"
              python3 -m ciel output --pdk-root "$PDK_ROOT" --pdk-family "$family" >/dev/null 2>&1
            }

            enable_pdk_if_missing() {
              local family="$1"
              local version="$2"

              if pdk_enabled "$family"; then
                return 0
              fi

              if [ -z "$version" ]; then
                version="$(resolve_latest_pdk_version "$family")"
                if [ -z "$version" ]; then
                  echo "Warning: could not resolve remote version for $family; leaving it missing"
                  return 0
                fi
                echo "Resolved latest $family PDK version: $version"
              fi

              echo "Fetching $family PDK via ciel (version: $version)..."
              if ! python3 -m ciel fetch --pdk-root "$PDK_ROOT" --pdk-family "$family" "$version"; then
                echo "Warning: ciel failed to fetch $family@$version"
                return 0
              fi

              if ! python3 -m ciel enable --pdk-root "$PDK_ROOT" --pdk-family "$family" "$version"; then
                echo "Warning: ciel failed to enable $family@$version after fetch"
              fi
            }

            enable_pdk_if_missing sky130 "$SKY130_PDK_VERSION"
            enable_pdk_if_missing gf180mcu "$GF180MCU_PDK_VERSION"
            enable_pdk_if_missing ihp-sg13g2 "$IHP_SG13G2_PDK_VERSION"

            echo "PDK_ROOT: $PDK_ROOT"
            echo "  sky130:     $(pdk_enabled sky130 && echo 'ok' || echo 'missing')"
            echo "  gf180mcu:   $(pdk_enabled gf180mcu && echo 'ok' || echo 'missing')"
            echo "  ihp-sg13g2: $(pdk_enabled ihp-sg13g2 && echo 'ok' || echo 'missing')"
          '';
        };
      }
    );
}
