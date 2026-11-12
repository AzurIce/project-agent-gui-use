// Only the GUI Use-owned test surface is touched; nothing is sent externally.
await android.app.launch({package_name: "dev.azurice.guiuse", activity: ".DiagnosticsActivity"});
await android.ui.wait({until: "window", value: "dev.azurice.guiuse", timeout_ms: 5000});
await android.ui.wait({until: "idle", timeout_ms: 5000});
function unique(screen, suffix) {
    if (screen.package_name !== "dev.azurice.guiuse") throw new Error("Not the owned test surface");
    const matches = screen.nodes.filter(node => node.view_id?.endsWith("/" + suffix));
    if (matches.length !== 1) throw new Error("Expected one " + suffix);
    return matches[0];
}
let screen = await android.ui.snapshot({filter: "all"});
const before = Number(unique(screen, "test_counter").text.split(":")[1]);
for (let i = 0; i < 3; i++) {
    const button = unique(screen, "test_increment");
    const result = await android.ui.act({node: button.id, action: "click"});
    if (!result.changed) throw new Error("Click not accepted");
    await android.ui.wait({until: "text", value: "Count: " + (before + i + 1), timeout_ms: 3000});
    screen = await android.ui.snapshot({filter: "all"});
}
return {before, after: unique(screen, "test_counter").text, clicks: 3, verified: true};
