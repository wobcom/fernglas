{ config, lib, pkgs, ... }:

{
  processes.prometheus = {
    exec = lib.concatStringsSep " " [
      "${pkgs.prometheus}/bin/prometheus"
      "--storage.tsdb.path=${config.env.DEVENV_STATE}/prometheus"
      "--config.file=${pkgs.writeText "prometheus.yml" (builtins.toJSON {
        global.scrape_interval = "5s";
        scrape_configs = [
          {
            job_name = "fernglas";
            static_configs = [
              {
                targets = [ "localhost:3000" ];
              }
            ];
          }
        ];
      })}"
      "--web.listen-address=[::]:9090"
    ];
  };
}
