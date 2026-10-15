self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.gpu-warden;
  settingsFormat = pkgs.formats.toml { };
in
{
  options.services.gpu-warden = {
    enable = lib.mkEnableOption "GPU Warden, the per-pin 12V-2x6 watchdog";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "gpu-warden.packages.\${system}.default";
      description = "The gpu-warden package to run.";
    };

    settings = lib.mkOption {
      type = settingsFormat.type;
      default = { };
      example = {
        shutdown = false;
        power_cap.percent = 70;
      };
      description = "Written to /etc/gpu-warden/config.toml. Unset keys keep their defaults; `gpu-warden config` prints them all.";
    };
  };

  config = lib.mkIf cfg.enable {
    boot.kernelModules = [ "i2c-dev" ];
    services.udev.packages = [ cfg.package ];
    environment.systemPackages = [ cfg.package ];
    environment.etc."gpu-warden/config.toml".source =
      settingsFormat.generate "gpu-warden.toml" cfg.settings;

    systemd.services.gpu-warden = {
      description = "GPU Warden 12V-2x6 watchdog";
      wantedBy = [ "multi-user.target" ];
      after = [
        "systemd-modules-load.service"
        "nvidia-persistenced.service"
      ];
      restartTriggers = [ config.environment.etc."gpu-warden/config.toml".source ];
      path = [ config.systemd.package ];

      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} run --config /etc/gpu-warden/config.toml";
        Restart = "always";
        RestartSec = 5;
        Nice = -5;
      };
    };
  };
}
