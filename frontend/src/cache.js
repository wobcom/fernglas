export let routers;
export const initCache = async () => {
	routers = await fetch("/api/routers").then(resp => resp.json());
};
