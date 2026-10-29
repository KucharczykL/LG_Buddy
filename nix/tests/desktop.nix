{
  self,
  home-manager,
  pkgs,
  package,
  desktop ? "gnome",
}:
let
  inherit (pkgs) lib;
  plasma = desktop == "plasma";
  configPath = "/home/lgtest/.config/lg-buddy/config.env";
  # A second real installed payload: keep the unchanged ELF contents but move
  # CLI, GTK wrapper and GUI together so actual process paths prove selection.
  changedPackage = pkgs.stdenvNoCC.mkDerivation {
    name = "lg-buddy-desktop-next";
    dontUnpack = true;
    nativeBuildInputs = [ pkgs.wrapGAppsHook4 ];
    buildInputs = [ pkgs.bash pkgs.gtk4 pkgs.libadwaita ];
    dontWrapGApps = true;
    installPhase = ''
      cp -a ${package}/. "$out/"
      chmod -R u+w "$out"
      rm "$out/bin/lg-buddy-gui"
      mv "$out/bin/.lg-buddy-gui-wrapped" "$out/bin/lg-buddy-gui"
      substituteInPlace "$out/share/applications/io.github.staphylococcus.LGBuddy.desktop" \
        "$out/lib/lg-buddy/kwin/setup.sh" \
        "$out/share/gnome-shell/extensions/lg-buddy-brightness@staphylococcus.github.io/extension.js" \
        --replace-fail ${lib.escapeShellArg (toString package)} "$out"
    '';
    preFixup = ''wrapGApp "$out/bin/lg-buddy-gui"'';
    meta = package.meta;
  };
  # Build the transport fixture separately: the installed runtime remains the
  # unmodified public package, including its ownership and source identity.
  fixture = package.overrideAttrs (_: {
    pname = "lg-buddy-desktop-tv-fixture";
    cargoBuildFlags = [
      "--package"
      "lg-buddy"
      "--example"
      "gui_journey_tv"
    ];
    doCheck = false;
    installPhase = ''
      runHook preInstall
      install -Dm755 target/*/release/examples/gui_journey_tv "$out/bin/gui_journey_tv"
      runHook postInstall
    '';
    postInstall = "";
    preFixup = "";
  });
  python = pkgs.python3.withPackages (p: [
    p.pyatspi
    p.dbus-python
    p.pygobject3
  ]);
  helpers = pkgs.runCommand "lg-buddy-desktop-ui" { } ''
    mkdir -p "$out"
    cp ${../../scripts/test-release-gui-accessibility.py} "$out/test-release-gui-accessibility.py"
    cp ${../../scripts/test-native-setup-recovery.py} "$out/test-native-setup-recovery.py"
  '';
  uiScript = pkgs.writeText "desktop-ui.py" (builtins.readFile ./desktop-ui.py);
in
assert builtins.elem desktop [
  "gnome"
  "plasma"
];
pkgs.testers.runNixOSTest {
  name = "lg-buddy-native-${desktop}";
  nodes.machine = { ... }: {
    imports = [
      self.nixosModules.lg-buddy
      home-manager.nixosModules.home-manager
    ];
    virtualisation = {
      memorySize = 6144;
      cores = 4;
      diskSize = 16384;
      useBootLoader = true;
      additionalPaths = [
        helpers
        pkgs.gobject-introspection
      ];
    };
    boot.loader.grub.enable = true;
    services.xserver.enable = true;
    services.displayManager.gdm.enable = !plasma;
    services.displayManager.sddm = lib.mkIf plasma {
      enable = true;
      wayland.enable = true;
    };
    services.desktopManager.gnome.enable = !plasma;
    # Test-only access to the real Shell launcher, following nixpkgs' GNOME
    # test convention. No production module enables this debugging interface.
    systemd.user.services."org.gnome.Shell@" = lib.mkIf (!plasma) {
      serviceConfig.ExecStart = [ "" "${pkgs.gnome-shell}/bin/gnome-shell --unsafe-mode" ];
    };
    services.desktopManager.plasma6.enable = plasma;
    services.displayManager.defaultSession = if plasma then "plasma" else "gnome";
    services.displayManager.autoLogin = {
      enable = true;
      user = "lgtest";
    };
    systemd.user.services.qualification-launcher = lib.mkIf plasma {
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      environment.GI_TYPELIB_PATH = "${pkgs.glib}/lib/girepository-1.0";
      serviceConfig.ExecStart = "${python}/bin/python ${./cached-launcher.py} /etc/profiles/per-user/lgtest/share/applications/io.github.staphylococcus.LGBuddy.desktop io.github.staphylococcus.NativeCachedLauncher";
    };
    networking.networkmanager.enable = true;
    users.users.lgtest = {
      isNormalUser = true;
      uid = 1000;
      password = "disposable";
    };
    services.lg-buddy = {
      enable = true;
      user = "lgtest";
      inherit package configPath;
      plasma.enable = plasma;
    };
    home-manager = {
      useGlobalPkgs = true;
      useUserPackages = true;
      users.lgtest = { lib, ... }: {
        imports = [ self.homeManagerModules.lg-buddy ];
        home.stateVersion = "26.05";
        services.lg-buddy = {
          enable = true;
          inherit package configPath;
          settings.screen.idle_timeout = 10;
          settings.system.sleep_wake_policy = "disabled";
        };
        dconf.settings = lib.mkIf (!plasma) {
          "org/gnome/desktop/interface".toolkit-accessibility = true;
          "org/gnome/desktop/session".idle-delay = lib.hm.gvariant.mkUint32 0;
          "org/gnome/desktop/screensaver".lock-enabled = false;
          "org/gnome/shell".welcome-dialog-last-shown-version = "999";
        };
      };
    };
    systemd.services.lg-buddy-tv-fixture = {
      wantedBy = [ "multi-user.target" ];
      before = [
        "LG_Buddy.service"
        "LG_Buddy_lifecycle.service"
      ];
      serviceConfig = {
        User = "lgtest";
        StateDirectory = "lg-buddy-test-tv";
        ExecStart = "${fixture}/bin/gui_journey_tv /var/lib/lg-buddy-test-tv";
      };
    };
    specialisation.changed.configuration = {
      services.lg-buddy.package = lib.mkForce changedPackage;
      home-manager.users.lgtest.services.lg-buddy = {
        package = lib.mkForce changedPackage;
        settings.screen.idle_timeout = lib.mkForce 20;
      };
    };
    specialisation.removed.configuration.home-manager.users.lgtest.services.lg-buddy.settings = {
      screen.idle_timeout = lib.mkForce null;
      system.sleep_wake_policy = lib.mkForce null;
    };
    environment.systemPackages = [
      python
      pkgs.at-spi2-core
      pkgs.glib
    ];
  };
  testScript = ''
    import json
    import shlex
    machine.start(allow_reboot=True)
    user = "runuser -u lgtest -- env XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
    cli = user + "${package}/bin/lg-buddy "
    ctl = user + "systemctl --user "
    observer = "env PYTHONPATH=${helpers} GI_TYPELIB_PATH=${pkgs.at-spi2-core}/lib/girepository-1.0:${pkgs.gobject-introspection}/lib/girepository-1.0:${pkgs.glib}/lib/girepository-1.0 ${python}/bin/python /tmp/desktop-ui.py "
    ui = user + observer
    token = "${builtins.dirOf configPath}/tvs/primary/access-token.json"
    base = machine.succeed("readlink -f /run/current-system").strip()
    changed = machine.succeed("readlink -f " + base + "/specialisation/changed").strip()
    # Test images boot a store path directly; establish the installer's initial
    # profile generation before exercising ordinary nix-env generation rollback.
    machine.succeed("nix-env --profile /nix/var/nix/profiles/system --set " + base)

    def shell_eval(code):
        reply = json.loads(machine.succeed(user + "busctl --user --json=short call org.gnome.Shell /org/gnome/Shell org.gnome.Shell Eval s " + shlex.quote(code)))
        success, result = reply["data"]
        assert success, result
        return json.loads(result) if result else None

    def session():
        machine.wait_for_unit("display-manager.service")
        machine.wait_for_file("/run/user/1000/wayland-0")
        machine.wait_until_succeeds(ctl + "is-active graphical-session.target LG_Buddy_screen.service", timeout=180)
        machine.copy_from_host("${uiScript}", "/tmp/desktop-ui.py")
        if ${if plasma then "True" else "False"}:
            machine.wait_until_succeeds(user + "busctl --user status io.github.staphylococcus.NativeCachedLauncher")
        else:
            machine.wait_until_succeeds(user + "busctl --user status org.gnome.Shell")
            assert shell_eval("(async () => { const {default: Shell} = await import('gi://Shell'); global.lgBuddyCachedLauncher = Shell.AppSystem.get_default().lookup_app('io.github.staphylococcus.LGBuddy.desktop'); return global.lgBuddyCachedLauncher !== null; })()")

    def assessment(status):
        machine.wait_until_succeeds(ui + "assessment " + status, timeout=90)

    def launch(expected="${package}"):
        machine.send_key("esc")
        if ${if plasma then "True" else "False"}:
            machine.succeed(user + "busctl --user call io.github.staphylococcus.NativeCachedLauncher /launcher io.github.staphylococcus.CachedLauncher Launch")
        else:
            assert shell_eval("global.lgBuddyCachedLauncher.activate(); true")
        machine.wait_until_succeeds(ui + "identity " + expected, timeout=60)

    def quit_gui():
        machine.succeed(user + "gapplication action io.github.staphylococcus.LGBuddy quit")
        machine.wait_until_fails(user + "busctl --user status io.github.staphylococcus.LGBuddy")

    def tv():
        return json.loads(machine.succeed("cat /var/lib/lg-buddy-test-tv/state.json"))

    with subtest("fresh public declaration starts the real session before pairing"):
        session()
        machine.wait_for_file("/var/lib/lg-buddy-test-tv/state.json")
        machine.fail("test -e ${configPath}")
        machine.fail("test -e " + token)
        lifecycle_pid = machine.succeed("systemctl show LG_Buddy_lifecycle.service -p MainPID --value")
        assert int(lifecycle_pid) > 0
        assessment("Incomplete")
        launch()
        print(machine.succeed(ui + "gate"))
        machine.screenshot("fresh-setup-gate")
        quit_gui()

    with subtest("normal CLI pairs with WebOS and the real verifier admits the GUI"):
        machine.succeed(cli + "setup --yes --non-interactive --tv-ip 127.0.0.1 --tv-mac 02:11:22:33:44:55 --input HDMI_3")
        assessment("Complete")
        assert machine.succeed("systemctl show LG_Buddy_lifecycle.service -p MainPID --value") == lifecycle_pid
        assert machine.succeed("systemctl show LG_Buddy_lifecycle.service -p NRestarts --value").strip() == "0"
        machine.wait_until_succeeds("test $(readlink -f /proc/" + lifecycle_pid.strip() + "/exe) = ${package}/bin/lg-buddy")
        machine.succeed("test -s " + token)
        assert tv()["pairing_prompt_count"] == 1, tv()
        saved_token = machine.succeed("sha256sum " + token)
        launch()
        print(machine.succeed(ui + "settings 10"))
        machine.screenshot("declared-settings")
        quit_gui()
        machine.fail(cli + "settings set screen.idle_timeout 15")
        machine.succeed(cli + "settings set screen.restore_policy aggressive")
        machine.succeed(cli + "settings set screen.honor_idle_inhibitors enabled")
        assert machine.succeed(cli + "settings get screen.restore_policy").strip() == "aggressive"
        machine.succeed(cli + "updates check")
        machine.fail(cli + "updates install")
        for suffix in ["service", "timer"]:
            machine.fail(ctl + "is-active LG_Buddy_update_check." + suffix)
            assert machine.succeed(ctl + "is-enabled LG_Buddy_update_check." + suffix + " || true").strip() == "masked"
        saved = machine.succeed("sha256sum ${configPath} " + token)

    with subtest("real desktop idle blanking and native input restore"):
        machine.succeed(user + "systemd-run --user --collect --unit=qualification-inhibitor " + observer + "inhibit")
        machine.wait_for_file("/tmp/qualification-inhibitor-ready")
        machine.send_key("esc")
        for _ in range(3):
            machine.sleep(5)
            assert tv()["screen_on"], tv()
        machine.succeed(ctl + "stop qualification-inhibitor.service")
        machine.sleep(5)
        assert tv()["screen_on"], tv()
        machine.wait_until_succeeds("${pkgs.jq}/bin/jq -e '.screen_on == false' /var/lib/lg-buddy-test-tv/state.json", timeout=45)
        machine.send_key("esc")
        machine.wait_until_succeeds("${pkgs.jq}/bin/jq -e '.screen_on == true' /var/lib/lg-buddy-test-tv/state.json", timeout=20)

    with subtest("activation changes effective policy without a TV power cycle"):
        before = tv()
        pid = machine.succeed(ctl + "show LG_Buddy_screen.service -p MainPID --value")
        machine.succeed("nix-env --profile /nix/var/nix/profiles/system --set " + changed)
        assert machine.succeed("readlink -f /nix/var/nix/profiles/system").strip() == changed
        machine.succeed(changed + "/bin/switch-to-configuration switch")
        machine.wait_until_succeeds(cli + "settings get screen.idle_timeout | ${pkgs.gnugrep}/bin/grep -x 20")
        assert machine.succeed(ctl + "show LG_Buddy_screen.service -p MainPID --value") != pid
        assert tv()["power_off_count"] == before["power_off_count"]
        assert machine.succeed("sha256sum ${configPath} " + token) == saved
        # A package switch restarts both managers' services. The user verifier
        # may have cached its initial snapshot before the system restart ends;
        # exercise the documented read-only recheck after activation settles.
        machine.wait_for_unit("LG_Buddy_lifecycle.service")
        machine.wait_until_succeeds(ctl + "is-active LG_Buddy_screen.service")
        machine.succeed(ui + "assessment Complete recheck")
        assessment("Complete")

    with subtest("cached desktop launcher selects the new CLI and real GTK GUI without logout"):
        launch("${changedPackage}")
        print(machine.succeed(ui + "settings 20"))
        quit_gui()

    with subtest("bootloader reboots into the selected generation with retained pairing"):
        boot = machine.succeed("cat /proc/sys/kernel/random/boot_id")
        machine.shutdown()
        machine.start(allow_reboot=True)
        session()
        assert machine.succeed("cat /proc/sys/kernel/random/boot_id") != boot
        assert machine.succeed("readlink -f /run/current-system").strip() == changed
        assert machine.succeed(cli + "settings get screen.idle_timeout").strip() == "20"
        assert machine.succeed("sha256sum ${configPath} " + token) == saved
        assessment("Complete")
        assert tv()["pairing_prompt_count"] == 0, tv()

    with subtest("generation rollback and declaration removal retain writable state"):
        machine.succeed("nix-env --profile /nix/var/nix/profiles/system --rollback")
        assert machine.succeed("readlink -f /nix/var/nix/profiles/system").strip() == base
        machine.succeed("/nix/var/nix/profiles/system/bin/switch-to-configuration switch")
        assert machine.succeed(cli + "settings get screen.idle_timeout").strip() == "10"
        launch()
        quit_gui()
        machine.succeed(base + "/specialisation/removed/bin/switch-to-configuration test")
        machine.succeed(cli + "settings set screen.idle_timeout 15")
        assert machine.succeed(cli + "settings get screen.idle_timeout").strip() == "15"
        assert machine.succeed("sha256sum " + token) == saved_token
        assessment("Complete")
        launch()
        print(machine.succeed(ui + "settings 15 editable"))
        machine.screenshot("undeclared-settings")
        quit_gui()

    print("QUALIFIED RUNTIME:", machine.succeed("${package}/bin/lg-buddy --version"))
    print("QUALIFIED OS:", machine.succeed("nixos-version"))
    print("QUALIFIED KERNEL:", machine.succeed("uname -r"))
    print("COMPONENT PACKAGES:", ${
      builtins.toJSON {
        compositor = if plasma then pkgs.kdePackages.kwin.version else pkgs.gnome-shell.version;
        gtk = pkgs.gtk4.version;
        libadwaita = pkgs.libadwaita.version;
        systemd = pkgs.systemd.version;
        qt = if plasma then pkgs.kdePackages.qtbase.version else "not applicable";
      }
    })
    print("QUALIFIED SESSION:", machine.succeed(ctl + "show-environment"))
    print("SYSTEM SERVICES:", machine.succeed("systemctl show LG_Buddy.service LG_Buddy_lifecycle.service -p MainPID -p ExecStart -p ActiveState"))
    print("USER SERVICE:", machine.succeed(ctl + "show LG_Buddy_screen.service -p MainPID -p ExecStart -p ActiveState"))
    ${lib.optionalString plasma ''
      print("LIVE KWIN BRIDGE:", machine.succeed(cli + "kwin-bridge check"))
    ''}
    print("TV OBSERVATION:", tv())
  '';
}
