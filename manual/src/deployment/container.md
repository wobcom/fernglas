# Deploying fernglas using OCI Containers / Docker

We have two different images. One image contains the UI, which is statically built and could be served from any other static webserver, i.e. nginx, apache, caddy. The other image contains the Fernglas software itself and is considered the backend. It exposes an HTTP API for the UI.

## Prequesits

+ OCI Runtime
    + You need to have installed Docker, podman, or any similar container daemon that can run OCI containers provided by us
+ You need to have a working reverse proxy setup
    + Fernglas only exposes HTTP. TLS and probably authentication needs to be handled by yourself. 
+ A Domain or Subdomain
    + Fernglas currently do not support path-based deployments, i.e. `myfancycompany.com/fernglas`.

## Fernglas Backend

```sh
docker pull ghcr.io/wobcom/fernglas:fernglas-0.1.0
```

You need to write a config file to specify Fernglas configuration. This needs to be put under `/config/config.yaml` in the standard configuration.
See the chapter on [configuration](configuration/README.md) for more information on how to write the collectors configuration.

## Fernglas Frontend

```sh
docker pull ghcr.io/wobcom/fernglas-frontend:fernglas-0.1.0
```

By setting `serve_static: true` in the config, the backend will also serve the bundled frontend files from the same webserver as the API.

You can take the fernglas-frontend image as base and serve the files with your own web server directly, if you want. The files need to be exposed at `/` of your domain, while the `/api/` path should be passed through to the API server.
