export let routers;
export let communities;
export const initCache = async () => {
	[routers, communities] = await Promise.all([
		fetch("/api/routers").then(resp => resp.json()),
		fetch("/communities.json").then(resp => resp.json())
	]);
};
