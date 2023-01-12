const routes = [];
let currentRoute;

export const route = (pattern, handler) => {
	routes.push({
		pattern: pattern,
		handler: handler
	});
};

export const go = (dest) => {
	window.location.hash = '#' + dest;
};

export const start = async () => {
	const dest = window.location.hash.slice(1);
	if (currentRoute && currentRoute.unload) currentRoute.unload();
	for (const route of routes) {
		const match = route.pattern.exec(dest);
		if (!match) continue;
		currentRoute = await route.handler(match.slice(1));
		return;
	}
};

window.addEventListener('hashchange', start);

