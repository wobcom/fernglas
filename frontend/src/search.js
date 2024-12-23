import { html, render } from 'lit-html';
import { go } from './router.js';
import { routers, tables } from './cache.js';

const modes = ["MostSpecific", "Exact", "OrLonger", "Contains"];

const formSubmit = (e) => {
	e.preventDefault();
	const data = new FormData(e.target);
	let val = data.get("input-field");
	const mode = data.get("query-mode");
	const router = data.get("router-sel");
	const table = data.get("table-sel");

	let res = "#/";
	let filter = [];
	if (val !== "") {
		res += `${mode}/${val}`;
	}
	if (router != "all") {
		filter.push(`Router=${router}`);
	}
	if (typeof table === "string" && table != "default" && table != "") {
		filter.push(`route_distinguisher=${table}`);
	}
	if (filter.length > 0) {
		res += "?" + filter.join("&");
	}

	window.location.hash = res;
	return false;
};

function deduplicated_routing_tables(tables) {
    return [...new Set(
        Object.values(tables)
            .flat()
            .map(JSON.stringify)
    )]
    .toSorted()
    .map(json_string => Array.from(JSON.parse(json_string)))
}

export const searchTemplate = ([ mode, ip, optionsString ]) => html`
	<form id="input" @submit=${formSubmit}>
		<select name="query-mode" id="query-mode" @change=${() => document.getElementById("input-submit").click()}>
			${modes.map(name => html`
				<option value=${name} ?selected=${mode === name}>${name}</option>
			`)}
		</select>
		<input name="input-field" id="input-field" type="text" spellcheck=false" autocomplete="new-password" autocorrect="off" autocapitalize="off" placeholder="Enter an IP address, prefix or DNS name..." value=${!!ip ? ip : ``} />
		<select name="router-sel" id="router-sel" @change=${() => document.getElementById("input-submit").click()}>
			<option value="all">on all</option>
			${[...new Set(routers.map(router => router[1].client_name))]
				.map(name => html`
				<option value=${name} ?selected=${(new URLSearchParams(optionsString)).get("Router") === name}>on ${name}</option>
			`)}
		</select>
        ${(deduplicated_routing_tables(tables).length > 1) ?
            html`
            <select name="table-sel" id="table-sel"
                @change=${() => document.getElementById("input-submit").click()}
                >
                ${deduplicated_routing_tables(tables).map(entry => {
                        let [rd, name] = entry
                        name = name ?? rd
                        return html`
                            <option value=${rd} ?selected=${(new URLSearchParams(optionsString)).get("route_distinguisher") === rd}>${name}</option>
                        `
                    })
                }
            </select>
        ` : `` }
		<input type="submit" id="input-submit" value="Go" />
	</form>
`;
