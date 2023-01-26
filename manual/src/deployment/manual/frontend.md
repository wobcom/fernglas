# Frontend and Reverse Proxy

Download the prebuilt frontend zipfile from our CI artifacts (link TBD).
Extract it to `/usr/local/share/fernglas-frontend`.

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
