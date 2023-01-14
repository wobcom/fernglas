import { html, render } from 'lit-html';
import { go } from './router.js';
import { searchTemplate } from './search.js';
import ndjsonStream from 'can-ndjson-stream';

const resultsTemplate = (query, results, done) => html`
	${searchTemplate(query)}

	<div class="results">
		${results.length > 0 ? html`
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
						<th>Local Pref</th>
						<th>Nexthop</th>
						<th>Status</th>
					</tr>
				</thead>
				<tbody>
					${results.map(result => html`
						<tr>
							<td><span>${result.from_client}</span></td>
							<td><span>${result.peer_address}</span></td>
							<td><span>${result.net}</span></td>
							<td><span>${result.as_path.join(" ")}</span></td>
							<td><span>${(result.large_communities || []).map(community => `(${community.join(",")})`).join(" ")}</span></td>
							<td><span>${result.origin}</span></td>
							<td><span>${result.med}</span></td>
							<td><span>${result.local_pref}</span></td>
							<td><span>${result.nexthop}</span></td>
							<td><span>${result.status}</span></td>
						</tr>
					`)}
				</tbody>
			</table>
		` : ''}
	</div>
	<div id="loading">
		${!done ? html`
			<div class="spinner"></div>
		` : ''}
	</div>
`;

const errorTemplate = (query, data) => html`
	${searchTemplate(query)}
	<div id="error">
		<h1 id="error-text">${data.text}</h1>
		<sub id="error-descr">${data.description}</sub>
	</div>
`;

const processResults = (results) => {

	// stage 1, combine pre- and post-policy adj-in tables
	// start out with PostPolicy
	const preAndPostPolicy = {};
	const preAndPostPolicyKey = route => `${route.from_client}:${route.peer_address}:${route.net}`;
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

	// stage 2, combine adj-in and loc-rib
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

	const [ mode, ip, prefixLength, optionsString ] = query;

	const searchParams = new URLSearchParams(optionsString);
	searchParams.append(mode, `${ip}/${prefixLength}`);

	render(resultsTemplate(query, [], false), document.getElementById('content'));
	const response = await fetch("/api/query?" + searchParams);
	if (!response.ok) {
		render(errorTemplate(query, {
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
		render(resultsTemplate(query, processResults(results), result.done), document.getElementById('content'));
	}
	if (results.length == 0) {
		render(errorTemplate(query, {
			text: "No data",
			description: "",
		}), document.getElementById('content'));
	}

	document.getElementById("input-field").focus();
};
