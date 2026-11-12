package dev.azurice.guiuse

import android.content.Context
import java.util.UUID

class AgentSettings(context: Context) {
    private val preferences = context.getSharedPreferences("connection", Context.MODE_PRIVATE)
    var serviceEnabled: Boolean
        get() = preferences.getBoolean("service_enabled", false)
        set(value) { preferences.edit().putBoolean("service_enabled", value).commit() }
    val configured: Boolean get() = runCatching { validate() }.isSuccess
    var endpoint: String
        get() = preferences.getString("endpoint", "wss://localhost:8765")!!
        set(value) { preferences.edit().putString("endpoint", value.trim()).apply() }
    var token: String
        get() = preferences.getString("token", "")!!
        set(value) { preferences.edit().putString("token", value.trim()).apply() }
    var certificateSha256: String
        get() = preferences.getString("pin", "")!!
        set(value) { preferences.edit().putString("pin", value.replace(":", "").lowercase().trim()).apply() }
    var allowedPackages: Set<String>
        get() = preferences.getStringSet("allowed", setOf(contextPackage))!!.toSet()
        set(value) { preferences.edit().putStringSet("allowed", value.toSet()).apply() }
    private val contextPackage = context.packageName
    val deviceId: String
        get() = preferences.getString("device_id", null) ?: UUID.randomUUID().toString().also {
            preferences.edit().putString("device_id", it).commit()
        }

    fun validate() {
        require(endpoint.startsWith("wss://")) { "Only wss:// endpoints are accepted" }
        require(token.length >= 32) { "A pairing token is required" }
        require(certificateSha256.matches(Regex("[0-9a-f]{64}"))) { "SHA-256 certificate fingerprint is required" }
        require(allowedPackages.isNotEmpty()) { "Allow at least one application" }
    }
}
