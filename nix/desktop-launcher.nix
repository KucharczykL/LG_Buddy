{
  lib,
  runCommand,
  desktop-file-utils,
  package,
  executable,
}:
let
  # Content-addressed glue stays identical across package and nixpkgs upgrades.
  # Keep user paths out of Exec's field-code parser (notably literal percent).
  launch = builtins.toFile "lg-buddy-launch" ''
    exec ${lib.escapeShellArg executable} "$@"
  '';
in
runCommand "lg-buddy-desktop-launcher"
  {
    nativeBuildInputs = [ desktop-file-utils ];
    # Override just the package's desktop file in the installed profile.
    meta.priority = (package.meta.priority or 5) - 1;
  }
  ''
    desktop_file="$out/share/applications/io.github.staphylococcus.LGBuddy.desktop"
    install -Dm644 ${package}/share/applications/io.github.staphylococcus.LGBuddy.desktop "$desktop_file"
    substituteInPlace "$desktop_file" \
      --replace-fail ${lib.escapeShellArg "Exec=${lib.getExe package}"} ${lib.escapeShellArg "Exec=/bin/sh ${launch}"}
    desktop-file-validate "$desktop_file"
  ''
