import { html, render } from 'lit-html';
import { go } from './router.js';

const formSubmit = (e) => {
	e.preventDefault();
	const data = new FormData(e.target);
	let val = data.get("input-field");
	if (val.indexOf("/") === -1) {
		if (val.indexOf(".") !== -1) {
			val += "/32";
		}
		if (val.indexOf(":") !== -1) {
			val += "/128";
		}
	}
	window.location.hash = `#/MostSpecific/${val}`;
	return false;
};

export const searchTemplate = (query) => html`
	<form id="input" @submit=${formSubmit}>
		<input name="input-field" id="input-field" type="text" spellcheck=false" autocomplete="new-password" autocorrect="off" autocapitalize="off" value=${`${query[1]}/${query[2]}`} />
	</form>
`;
