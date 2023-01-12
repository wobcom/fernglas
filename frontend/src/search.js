import { html, render } from 'lit-html';
import { go } from './router.js';

const keyPressed = (e) => {
	const which = e.which || e.keyCode;
	if (which == 13) {
		let val = e.target.value;
		if (val.indexOf("/") === -1) {
			if (val.indexOf(".") !== -1) {
				val += "/32";
			}
			if (val.indexOf(":") !== -1) {
				val += "/128";
			}
		}
		window.location.hash = `#/MostSpecific/${val}`;
	}
};

export const searchTemplate = (query) => html`
	<div id="input">
		<input id="input-field" type="text" spellcheck=false" autocomplete="new-password" autocorrect="off" autocapitalize="off" @keypress=${keyPressed} value=${query} />
	</div>
`;
