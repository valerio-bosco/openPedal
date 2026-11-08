{
  description = "openPedal - real-time guitar effects";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          cargo
          rustc
          clippy
          rustfmt
          rust-analyzer
          pkg-config
          qpwgraph # to see the audio connections
          nodejs # only needed to develop the UI (ui/); not needed to run openPedal
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
