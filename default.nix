{ pkgs ? import <nixpkgs> {} }:

pkgs.rustPlatform.buildRustPackage rec {
  pname = "evremap";
  version = "2024-03-09";

  src = pkgs.lib.cleanSource ./.;
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = with pkgs; [
    libevdev
    pkg-config
  ];

  cargoHash = pkgs.lib.fakeHash;
  cargoBuildFlags = [ "--release" "--all-features" ];

  RUSTUP_TOOLCHAIN = "stable";
  PKG_CONFIG_PATH = "${pkgs.libevdev}/lib/pkgconfig";

  meta = with pkgs.lib; {
    description = "A keyboard input remapper for Linux/Wayland systems, written by @wez";
    homepage = https://github.com/wez/evremap;
    license = licenses.mit;
    maintainers = [ maintainers.wez ];
    platforms = platforms.all;
  };
}
