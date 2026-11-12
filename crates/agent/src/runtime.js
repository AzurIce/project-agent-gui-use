"use strict";
const state = JSON.parse(__state);
const android = {app: {}, ui: {}, notification: {}, host: {}};
const catalog = JSON.parse(__catalog);
for (const {name} of catalog) {
    const [group, method] = name.split(".");
    android[group][method] = async (params = {}) => {
        const response = JSON.parse(__rpc(name, JSON.stringify(params)));
        if (response.error) {
            const error = new Error(response.error.message);
            error.code = response.error.code;
            throw error;
        }
        return response.result;
    };
}
for (const name of ["devices", "events"]) {
    android.host[name] = async () => {
        const response = JSON.parse(__rpc("host." + name, "{}"));
        if (response.error) throw new Error(response.error.message);
        return response.result;
    };
}
android.help = (method) => method === undefined ? catalog : JSON.parse(__help(method));
const print = (...values) => __print(values.map(value =>
    typeof value === "string" ? value : JSON.stringify(value)).join(" "));
const console = Object.freeze({log: print});
for (const group of [android.app, android.ui, android.notification, android.host, android]) {
    Object.freeze(group);
}
