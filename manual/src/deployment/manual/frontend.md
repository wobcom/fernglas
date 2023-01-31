# Frontend and Reverse Proxy

Download the prebuilt frontend tar.
Extract it to `/usr/local/share/fernglas-frontend`.

```
$ sudo mkdir -p /usr/local/share/fernglas-frontend
$ wget -O- https://github.com/wobcom/fernglas/releases/download/fernglas-0.1.0/fernglas-frontend-0.1.0.tar.xz | sudo tar -C /usr/local/share/fernglas-frontend -xJ
```

Set up your reverse proxy / webserver.
A configuration for nginx might look like this:

```
server {
	# we expect that you know how to set up a secure web server on your platform

	location / {
		root /usr/local/share/fernglas-frontend;
	}
	location /api/ {
		proxy_pass http://[::1]:3000; # match the api.bind setting from your fernglas config
		proxy_set_header Host $host;
		proxy_set_header X-Real-IP $remote_addr;
		proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
		proxy_set_header X-Forwarded-Proto $scheme;
		proxy_set_header X-Forwarded-Host $host;
		proxy_set_header X-Forwarded-Server $host;
	}
}
```
