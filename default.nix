{ pkgs ? import <nixpkgs> {} }:

pkgs.rustPlatform.buildRustPackage rec {
  pname = "evremap";
  version = "1.0";

  src = pkgs.lib.cleanSource ./.;
  cargoLock.lockFile = ./Cargo.lock;

  buildInputs = with pkgs; [
    rustup
  ];
  propagatedBuildInputs = with pkgs; [
    libevdev
  ];

  phases = "installPhase";

  installPhase = ''
mkdir -p $out/bin
cp $src/target/release/evremap $out/bin/evremap
  '';

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
