import { route, go, start } from './router.js';
import { historyView } from './historyView.js';
import { resultsView } from './resultsView.js';
import { hideDiv } from './helpers.js';

(async () => {
	// read settings from indexeddb
	//await initDataStorage();
	//await initSettings();

	route(/^\/$/, historyView);
	route(/^\/([^\/]+)\/([^\/]+)\/([^\/]+)$/, resultsView);

	document.getElementById('loader-overlay').innerHTML = '';
	hideDiv('loader-overlay');
	if (!window.location.hash.length) go('/');
	start();
}) ()
