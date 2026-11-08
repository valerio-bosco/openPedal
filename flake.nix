{
  description = "openPedal - real-time guitar effects";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };
      # Stable Rust with the iPad targets' std (cross-compiled staticlib, see apps/ios/README.md).
      rust = pkgs.rust-bin.stable.latest.default.override {
        extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
        targets = [ "aarch64-apple-ios" "aarch64-apple-ios-sim" ];
      };
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          rust # cargo, rustc, clippy, rustfmt, rust-analyzer + iOS std
          pkg-config
          qpwgraph # to see the audio connections
          nodejs # only needed to develop the UI (apps/desktop/ui/); not needed to run openPedal
          git-cliff # changelogs, used by tools/release.sh
        ];
        # libjack2 is only needed to build (headers and jack.pc); at runtime
        # PipeWire's libjack wins thanks to LD_LIBRARY_PATH, like `pw-jack` does.
        buildInputs = [ pkgs.libjack2 ];

        # openPedal speaks JACK: PipeWire's libjack connects it to the PipeWire graph.
        # (The buffer size is set in openpedal.toml, not here.)
        LD_LIBRARY_PATH = "${pkgs.pipewire.jack}/lib";
      };
    };
}
