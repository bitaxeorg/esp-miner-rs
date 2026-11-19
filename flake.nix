{
  description = "esp-miner-rs: Rust firmware for the Bitaxe boards";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Xtensa `esp` Rust toolchain and the xtensa/riscv32 GCC, rustc version selected below. The
    # esp-hal 1.x crates need rustc 1.95 or newer.
    esp-rs-nix = {
      url = "github:leighleighleigh/esp-rs-nix";
      flake = false;
    };
  };

  outputs =
    { nixpkgs, esp-rs-nix, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAllSystems (
        pkgs:
        let
          esp-rs = pkgs.callPackage "${esp-rs-nix}/package.nix" {
            rustc-version = "1.96.0.0";
          };
        in
        {
          default = pkgs.mkShell {
            packages = [
              esp-rs
              # resolves `channel = "esp"` of firmware/rust-toolchain.toml through RUSTUP_TOOLCHAIN
              pkgs.rustup
              pkgs.espflash
              # cargo runner of firmware/.cargo/config.toml
              pkgs.probe-rs-tools
              pkgs.rust-analyzer
              pkgs.pkg-config
              pkgs.just
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              # libudev
              pkgs.systemdMinimal
            ];

            # rustup uses the esp toolchain from the Nix store, for the firmware and the host
            # tests alike, instead of looking for an `esp` toolchain in ~/.rustup.
            RUSTUP_TOOLCHAIN = "${esp-rs}";

            PROBE_RS_PROTOCOL = "jtag";
            PROBE_RS_CHIP = "esp32s3";
          };
        }
      );
    };
}
