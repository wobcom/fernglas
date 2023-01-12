import { showDiv, hideDiv } from './helpers.js';
import { html, render } from 'lit-html';

export const showAlertModal = (text) => {
	showDiv('overlay');
	return new Promise(resolve => {
		render(html`
			<div class="modal">
				<div class="box alert">
					${text}
					<div class="button" @click=${() => { hideOverlay(); resolve(); }}>OK</div>
				</div>
			</div>
		`, document.getElementById('overlay'));
	});
};

export const showSelectModal = (content) => {
	showDiv('overlay');
	return new Promise(resolve => {
		render(html`
			<div class="modal">
				<div class="box select">
					${content}
					<a @click=${() => { hideOverlay(); resolve(); }}>Close</a>
				</div>
			</div>
		`, document.getElementById('overlay'));
	});
};

export const showModal = (title, content) => {
	showDiv('overlay');
	return new Promise(resolve => {
		render(html`
			<div class="modal-dialog">
				<div id="modal-content" class="modal-content">
					<div class="modal-header">
						<div class="modal-close" @click=${() => { hideOverlay(); resolve(); }}></div>
						<h4 class="modal-title">${title}</h4>
					</div>
					<div class="modal-body">${content}</div>
				</div>
			</div>
		`, document.getElementById('overlay'));
	});
};

export const showLoader = () => {
	showDiv('overlay');
	render(html`
		<div class="loading">
			<div class="spinner"></div>
		</div>
	`, document.getElementById('overlay'));
};

export const hideOverlay = () => hideDiv('overlay');

