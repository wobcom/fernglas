import { route, go, start } from './router.js';
import { historyView } from './historyView.js';
import { resultsView } from './resultsView.js';
import { hideDiv } from './helpers.js';
import { initCache } from './cache.js';

(async () => {
	// read settings from indexeddb
	//await initDataStorage();
	//await initSettings();
	await initCache();

	route(/^\/(\?.*)?$/, historyView);
	route(/^\/([^\/]+)\/([^?]+)(\?.*)?$/, resultsView);

	document.getElementById('loader-overlay').innerHTML = '';
	hideDiv('loader-overlay');
	if (!window.location.hash.length) go('/');
	start();
}) ()
