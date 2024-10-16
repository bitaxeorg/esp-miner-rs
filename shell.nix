{ pkgs ? import <nixpkgs> {}}:
let 
    esp-rs-src = builtins.fetchTarball "https://github.com/Georges760/esp-rs-nix/archive/v1.81.tar.gz";
    esp-rs = pkgs.callPackage "${esp-rs-src}/esp-rs/default.nix" {};
in
pkgs.mkShell rec {
    name = "esp-rs-nix";

    buildInputs = [
        esp-rs
        pkgs.rustup # I can't remember why I needed this 
        #pkgs.espflash # broken, install with cargo instead!
        pkgs.rust-analyzer
        pkgs.pkg-config 
        pkgs.stdenv.cc 
        pkgs.systemdMinimal
        pkgs.just
    ];

    shellHook = ''
    # optionally reload your bashrc to make the ugly nix-shell prompt go away 
    #. ~/.bashrc

    # this is important - it tells rustup where to find the esp toolchain,
    # without needing to copy it into your local ~/.rustup/ folder.
    export RUSTUP_TOOLCHAIN=${esp-rs}
    '';
}