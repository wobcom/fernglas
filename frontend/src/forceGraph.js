import { html } from 'lit-html';
import { forceY, forceManyBody } from "d3-force";

let el, graph, hoverNode, maxLevel, firstNode = true, routeNodes = [];

const renderNode = (node, color, ctx, globalScale) => {
        const label = node.label.split('\n');
        if (label.length < 2) label.push('');
        const fontSize = 4;
        ctx.font = `${fontSize}px sans-serif`;
        const textWidth = Math.max(ctx.measureText(label[0]).width, ctx.measureText(label[1]).width);
        const bckgDimensions = [textWidth, fontSize * 2].map(n => n + fontSize * 0.6); // some padding

        // shadow canvas
        if (color) {
                ctx.fillStyle = color;
                ctx.fillRect(node.x - bckgDimensions[0] / 2, node.y - bckgDimensions[1] / 2, ...bckgDimensions);
                return;
        }

        ctx.fillStyle = 'rgba(72, 48, 72, .6)';
        if (node.hover) ctx.fillStyle = 'rgba(60, 30, 50, 1)';
        ctx.lineWidth = 0.4;
        ctx.fillRect(node.x - bckgDimensions[0] / 2, node.y - bckgDimensions[1] / 2, ...bckgDimensions);
        ctx.fillStyle = 'rgba(255, 255, 255, .8)';
        ctx.fillRect(node.x - bckgDimensions[0] / 2, node.y - .2, bckgDimensions[0], .4);

        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillStyle = 'rgba(255, 255, 255, .8)';
        if (node.hover) ctx.fillStyle = 'rgba(255, 255, 255, 1)';
        ctx.fillText(label[0], node.x, node.y - fontSize + fontSize / 2);
        ctx.fillText(label[1], node.x, node.y + fontSize / 1.5);
};

const renderLink = (link, ctx, globalScale) => {
        const start = link.source;
        const end = link.target;

        if (!start || !end || !start.hasOwnProperty('x') || !end.hasOwnProperty('x')) return; // skip invalid link

        const offset = (node) => (link.route_id - node.min_route_id) / 2 - (node.max_route_id - node.min_route_id) / 4;

        if (firstNode) {
                firstNode = false;
                routeNodes = [ start ];
                ctx.moveTo(start.x + offset(start), start.y)
        }

        ctx.bezierCurveTo(
                start.x + offset(start) - 20, start.y,
                end.x + offset(end) - 20, end.y,
                end.x + offset(end), end.y
        );

        routeNodes.push(end);

        if (link.last) {
                ctx.strokeStyle = link.color + (link.primary ? "ff" : "40");
                ctx.lineWidth = link.primary ? 0.6 : 0.3;
                if (hoverNode) {
                        if (routeNodes.includes(hoverNode)) {
                                ctx.strokeStyle = link.color + (link.primary ? "ff" : "80");
                        } else {
                                ctx.strokeStyle = "rgba(230, 200, 250, .1)";
                        }
                }
                ctx.stroke();
                ctx.beginPath();
                firstNode = true;
        }
};


export const setData = routes => {

	const data = {
		nodes: [],
		links: []
	};

	let nodeId = 0;
	let nodes = {};

	const addNode = (name, level, route_id) => {
		if (!nodes[name]) {
			nodes[name] = {
				id: nodeId,
				label: name,
				level: level,
				min_route_id: route_id,
				max_route_id: route_id,
			};
			nodeId++;
		} else {
			nodes[name].level = Math.max(nodes[name].level, level);
			nodes[name].min_route_id = Math.min(nodes[name].min_route_id, route_id);
			nodes[name].max_route_id = Math.max(nodes[name].max_route_id, route_id);
		}
		return nodes[name].id;
	};
	let routeIds = new Set();

	maxLevel = 0;
	console.log(routes[0]);
	for (let route of routes) {
		const routeIdStr = `${route.from_client}:${route.net}:${JSON.stringify(route.as_path)}:${JSON.stringify(route.large_communities)}:${route.nexthop}`;
		routeIds.add(routeIdStr);
		const route_id = Array.from(routeIds).indexOf(routeIdStr);
		let level = 0, lastNode;
		const routeNodes = [route.from_client, ...route.as_path.map(x => x.toString()), route.net];
		for (let name of routeNodes) {
			const id = addNode(name, level, route_id);
			if (lastNode !== undefined) {
				data.links.push({
					source: lastNode,
					target: id,
					primary: route.status === "Selected",
					route_id,
				});
			}
			lastNode = id;

			maxLevel = Math.max(maxLevel, level);
			level++;
		}
		data.links[data.links.length-1].last = true;
	}

	data.nodes = Object.values(nodes);

	console.log(data);

	graph(el)
		.d3Force('y1', forceY(-100).strength(node => 0.5 * (maxLevel - node.level - 2)))
		.d3Force('y2', forceY(100).strength(node => 0.5 * node.level))
		.d3Force('charge', forceManyBody().strength(node => -150))
		.graphData(data);

	setTimeout(() => {
		graph.zoomToFit(50, 100)
		//hide(loader);
	}, 200);
	graph.width(window.innerWidth);
	graph.height(window.innerHeight);

};

export const init = async (element) => {
	el = element;

	const module = await import("force-graph/dist/force-graph.min.js");
	const ForceGraph = module.default;
	graph = ForceGraph()
		.onNodeHover((node, prev) => {
			if (prev) {
				prev.hover = false;
				hoverNode = null;
			}
			if (node) {
				node.hover = true;
				hoverNode = node;
			}
		})
		.onRenderFramePost((ctx, scale) => {
			if (hoverNode) renderNode(hoverNode, null, ctx, scale);
		})
		.nodeCanvasObject((node, ctx, globalScale) => renderNode(node, null, ctx, globalScale))
		.nodePointerAreaPaint(renderNode)
		.linkAutoColorBy('route_id')
		.linkCanvasObject(renderLink);
};
