import { html, render } from 'lit-html';
import { go } from './router.js';
import { searchTemplate } from './search.js';
import ndjsonStream from 'can-ndjson-stream';

const resultsTemplate = (query, results, done) => html`
	${searchTemplate(query)}

	<div class="results">
		<table>
			<thead>
				<tr>
					<th>Router</th>
					<th>Peer</th>
					<th>Prefix</th>
					<th>AS Path</th>
					<th>Large Communities</th>
					<th>Origin</th>
					<th>MED</th>
					<th>Nexthop</th>
					<th>Status</th>
				</tr>
			</thead>
			<tbody>
				${results.map(result => html`
					<tr>
						<td><span>${result.from_client}</span></td>
						<td><span>${result.remote_router_id}</span></td>
						<td><span>${result.net}</span></td>
						<td><span>${result.as_path.join(" ")}</span></td>
						<td><span>${(result.large_communities || []).map(community => `(${community.join(",")})`).join(" ")}</span></td>
						<td><span>${result.origin}</span></td>
						<td><span>${result.med}</span></td>
						<td><span>${result.nexthop}</span></td>
						<td><span>${result.status}</span></td>
					</tr>
				`)}
				${!done ? html`<tr><td>Loading...</td></tr>` : ''}
			</tbody>
		</table>
	</div>
`;

const errorTemplate = (data) => html`
	${searchTemplate()}
	<div id="error">
		<h1 id="error-text">${data.text}</h1>
		<sub id="error-descr">${data.description}</sub>
	</div>
`;

const processResults = (results) => {
	// start out with PostPolicy
	const preAndPostPolicy = {};
	const preAndPostPolicyKey = route => `${route.from_client}:${route.remote_router_id}:${route.net}`;
	for (let route of results) {
		if (route.table === "PostPolicyAdjIn") {
			route.status = "Accepted";
			preAndPostPolicy[preAndPostPolicyKey(route)] = route;
		}
	}
	// add routes which are _only_ in PrePolicy => have not been accepted
	for (let route of results) {
		if (route.table === "PrePolicyAdjIn") {
			route.status = "";
			const key = preAndPostPolicyKey(route);
			if (!preAndPostPolicy[key]) {
				preAndPostPolicy[key] = route;
			}
		}
	}
	const all = {};
	const allKey = route => `${route.from_client}:${route.net}:${JSON.stringify(route.as_path)}:${JSON.stringify(route.large_communities)}:${route.nexthop}`;
	for (let route of Object.values(preAndPostPolicy)) {
		all[allKey(route)] = route;
	}
	for (let route of results) {
		if (route.table === "LocRib") {
			route.status = "Selected";
			const key = allKey(route);
			if (all[key])
				all[key].status = "Selected";
			else
				all[key] = route;
		}
	}
	const newResults = Object.values(all);
	newResults.sort((a, b) => {
		let res;
		res = a.net.localeCompare(b.net);
		if (res !== 0) return res;
		const statusRank = [ "Selected", "Accepted", "" ];
		res = statusRank.indexOf(a.status) - statusRank.indexOf(b.status);
		if (res !== 0) return res;
		return 0;
	});
	return newResults;
};

export const resultsView = async (query) => {

	const searchParams = {};
	searchParams[query[0]] = `${query[1]}/${query[2]}`;

	const response = await fetch("/query?" + new URLSearchParams(searchParams));
	if (!response.ok) {
		render(errorTemplate({
			text: "No data",
			description: await response.text(),
		}), document.getElementById('content'));
		return;
	}

	const reader = ndjsonStream(response.body).getReader();
	let result;
	let results = [];
	while (!result || !result.done) {
		result = await reader.read();
		if (result.value) {
			results.push(result.value);
		}
		render(resultsTemplate(`${query[1]}/${query[2]}`, processResults(results), result.done), document.getElementById('content'));
	}
};
