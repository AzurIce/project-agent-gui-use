// Full snapshots stay local; only relevant fields enter model context.
const screen = await android.ui.snapshot({filter: "all"});
return {
    package: screen.package_name,
    activity: screen.activity,
    truncated: screen.truncated,
    total_nodes: screen.nodes.length,
    controls: screen.nodes
        .filter(node => node.actions.length && (node.text || node.desc || node.view_id))
        .slice(0, 15)
        .map(({id, text, desc, view_id, actions}) => ({id, text, desc, view_id, actions}))
};
