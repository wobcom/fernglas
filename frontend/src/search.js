import { html, render } from 'lit-html';
import { go } from './router.js';

const formSubmit = (e) => {
	e.preventDefault();
	const data = new FormData(e.target);
	let val = data.get("input-field");
	const mode = data.get("query-mode");
	if (val.indexOf("/") === -1) {
		if (val.indexOf(".") !== -1) {
			val += "/32";
		}
		if (val.indexOf(":") !== -1) {
			val += "/128";
		}
	}
	window.location.hash = `#/${mode}/${val}`;
	return false;
};

export const searchTemplate = (query) => html`
	<form id="input" @submit=${formSubmit}>
		<select name="query-mode" id="query-mode" @change=${() => document.getElementById("input-submit").click()}>
			${["MostSpecific", "Exact", "OrLonger", "Contains"].map(name => html`
				<option value=${name} ?selected=${query[0] === name}>${name}</option>
			`)}
		</select>
		<input name="input-field" id="input-field" type="text" spellcheck=false" autocomplete="new-password" autocorrect="off" autocapitalize="off" value=${query ? `${query[1]}/${query[2]}` : ''} />
		<input type="submit" id="input-submit" value="Go" />
	</form>
`;
