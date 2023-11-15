import { html, render } from 'lit-html';
import { go } from './router.js';
import { searchTemplate } from './search.js';

const historyTemplate = (routers, optionsString) => html`
	${searchTemplate(routers, [ null, null, null, optionsString ])}
`;

export const historyView = async ([ optionsString ]) => {
	const routers = await fetch("/api/routers").then(resp => resp.json());
	render(historyTemplate(routers, optionsString), document.getElementById('content'));
	document.getElementById("input-field").focus();
};
