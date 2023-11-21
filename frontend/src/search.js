import { html, render } from 'lit-html';
import { go } from './router.js';
import { routers } from './cache.js';

const modes = ["MostSpecific", "Exact", "OrLonger", "Contains"];

const formSubmit = (e) => {
	e.preventDefault();
	const data = new FormData(e.target);
	let val = data.get("input-field");
	const mode = data.get("query-mode");
	const router = data.get("router-sel");
	let res = "#/";
	if (val !== "") {
		res += `${mode}/${val}`;
	}
	if (router != "all") {
		res += `?Router=${router}`;
	}
	window.location.hash = res;
	return false;
};

export const searchTemplate = ([ mode, ip, prefixLength, optionsString ]) => html`
	<form id="input" @submit=${formSubmit}>
		<select name="query-mode" id="query-mode" @change=${() => document.getElementById("input-submit").click()}>
			${modes.map(name => html`
				<option value=${name} ?selected=${mode === name}>${name}</option>
			`)}
		</select>
		<input name="input-field" id="input-field" type="text" spellcheck=false" autocomplete="new-password" autocorrect="off" autocapitalize="off" value=${!!ip && !!prefixLength ? `${ip}/${prefixLength}` : ``} />
		<select name="router-sel" id="router-sel" @change=${() => document.getElementById("input-submit").click()}>
			<option value="all">on all</option>
			${[...new Set(Object.values(routers).map(router => router.client_name))].map(name => html`
				<option value=${name} ?selected=${(new URLSearchParams(optionsString)).get("Router") === name}>on ${name}</option>
			`)}
		</select>
		<input type="submit" id="input-submit" value="Go" />
	</form>
`;
