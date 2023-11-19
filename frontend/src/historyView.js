import { html, render } from 'lit-html';
import { go } from './router.js';
import { searchTemplate } from './search.js';

const historyTemplate = (optionsString) => html`
	${searchTemplate([ null, null, null, optionsString ])}
`;

export const historyView = async ([ optionsString ]) => {
	render(historyTemplate(optionsString), document.getElementById('content'));
	document.getElementById("input-field").focus();
};
