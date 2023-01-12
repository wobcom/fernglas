export const showDiv = (id) => {
	const element = document.getElementById(id);
	if (element) element.classList.remove('hidden');
};

export const hideDiv = (id) => {
	const element = document.getElementById(id);
	if (element) element.classList.add('hidden');
};
