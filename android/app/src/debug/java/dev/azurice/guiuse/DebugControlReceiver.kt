package dev.azurice.guiuse

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Debug-only, restricted to shell/system callers holding DUMP permission. */
class DebugControlReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.getStringExtra("command")) {
            "configure" -> {
                val settings = AgentSettings(context)
                settings.endpoint = intent.getStringExtra("endpoint") ?: return
                settings.token = intent.getStringExtra("token") ?: return
                settings.certificateSha256 = intent.getStringExtra("pin") ?: return
                settings.allowedPackages = setOf(context.packageName)
                settings.validate()
                context.startForegroundService(Intent(context, ConnectionService::class.java))
            }
            "disconnect" -> context.stopService(Intent(context, ConnectionService::class.java))
            "fixture" -> context.startActivity(Intent(context, DiagnosticsActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK).putExtra("sensitive", intent.getBooleanExtra("sensitive", false)))
            "post" -> TestNotifications.post(context)
            "cancel" -> TestNotifications.cancel(context)
            "clear" -> {
                context.stopService(Intent(context, ConnectionService::class.java))
                context.getSharedPreferences("connection", Context.MODE_PRIVATE).edit().remove("token").remove("pin").apply()
            }
        }
    }
}
