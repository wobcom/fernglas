import { html, render } from 'lit-html';
import { go } from './router.js';
import { searchTemplate } from './search.js';

const historyTemplate = () => html`
	${searchTemplate()}
`;

export const historyView = async () => {
	render(historyTemplate(), document.getElementById('content'));
};
