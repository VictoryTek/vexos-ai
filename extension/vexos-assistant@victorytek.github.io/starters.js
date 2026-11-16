// The starter tiles. Each is an id the app knows: the extension sends only the
// id, and the app maps it to a fixed prompt that it puts in the input as a
// draft. Nothing is sent until the user clicks Send there.
//
// The ids and titles are the same list as crates/vexos-ai-viewmodel/src/starters.rs;
// tests/extension/starters.test.mjs fails if the two drift apart.

export const STARTERS = [
    {id: 'diagnose', title: 'Diagnose a problem', icon: 'dialog-question-symbolic'},
    {id: 'change', title: 'Change this system', icon: 'preferences-system-symbolic'},
    {id: 'explain', title: 'Explain the last rebuild', icon: 'document-open-recent-symbolic'},
];

export const isStarter = id => typeof id === 'string' && STARTERS.some(s => s.id === id);
