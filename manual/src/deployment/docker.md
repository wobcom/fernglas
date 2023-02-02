# Deploying fernglas using Docker

Since we know, that not the whole word is using Nix, we also provide some Docker images.
But you really should give NixOS a try.

We have two different images. One image serves the UI, which is statically built and could be served from any other static webserver, i.e. nginx, apache, caddy. The other image contains the Fernglas software itself and is considered the backend. It exposes an HTTP API for the UI.

## Prequesits

+ Docker
    + You need to have installed Docker or a similar container daemon - podman and others will do the job, too.
+ You need to have a working reverse proxy setup
    + Fernglas only exposes HTTP and TLS and probably authentication needs to be handled by yourself. 
+ A Domain or Subdomain
    + Fernglas currently do not support path-based deployments, i.e. `myfancycompany.com/fernglas`.

## Fernglas Backend

You need to write a config file to specify Fernglas configuration. This needs to be put under `/config/config.yaml` in the standard configuration.
The standard configuration uses port `3000` and `11019` to expose the API and collect the BMP data stream. You can change those ports, if needed,
but you need to expose 11019 to an IP address reachable from your router, probably bind this port to `[::]:11019` and check for outside reachability.
Note: You also need to specify the IP addresses of possible peers in the config file to ensure no unauthorized person is steaming a BMP stream to your machine.
The API port must be proxied by a reverse proxy and needs to be exposed at `/api` of your domain.

## Fernglas Frontend

We packed a HTTP server into this docker image which servers the static files - which are built from Fernglas Frontend - on port 8000. 
Those files need to be exposed at `/` of your domain.

You can take those files and serve them from the Reverse Proxy directly, if you want.

## Example Docker-Compose Configuration

Note: docker-compose is not considered a tool for production, you may need to work out a deployment for yourself in a Kubernetes or bare Docker environment, but this contains everything you need.

```yaml
version: "3"

services:
  fernglas:
    image: ghcr.io/wobcom/fernglas:latest
    volumes:
      # Mount with read-only configuration file
      - "./config.yml:/config/config.yml:ro"
    ports: 
      # API port - only used from reverse proxy
      - "127.0.0.1:3000:3000"
      # Port for BMP stream collection
      - "11019:11019"
    networks:
      - reverse-proxy
  fernglas-frontend:
    image: ghcr.io/wobcom/fernglas-frontend:latest
    ports: 
      # Web port - only used from reverse proxy
      - "127.0.0.1:8000:8000"
    networks:
      - reverse-proxy
    

networks:
  # This network needs also be attached to the reverse proxy, if it runs in Docker.
  # If not, this can be omitted and Fernglas can use the default network.
  reverse-proxy:
    name: reverse-proxy
    external: true

```

Your reverse proxy still needs to pass `/api` to `localhost:3000` and `/` to `localhost:8000` and do some TLS termination.