package dev.azurice.guiuse

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Debug-only, restricted to shell/system callers holding DUMP permission. */
class DebugControlReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.getStringExtra("command")) {
            "capabilities" -> { resultCode = 1; resultData = "usb-import" }
            "import" -> {
                val file = java.io.File(context.filesDir, "usb-pairing.json")
                try {
                    require(file.length() in 1..65536)
                    val value = org.json.JSONObject(file.readText())
                    val settings = AgentSettings(context)
                    settings.endpoint = value.getString("endpoint")
                    settings.token = value.getString("token")
                    settings.certificateSha256 = value.getString("certificate_sha256")
                    // USB development entry: don't make the user maintain a package
                    // allowlist just to try ordinary apps. Manual/test pairing keeps
                    // its existing policy; platform permission checks still apply.
                    val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
                    settings.allowedPackages = context.packageManager.queryIntentActivities(launcher, 0)
                        .map { it.activityInfo.packageName }.toSet() + context.packageName
                    settings.validate()
                    ConnectionService.start(context)
                    resultCode = 1; resultData = settings.deviceId
                } catch (_: Exception) { resultCode = 0; resultData = "USB configuration failed" }
                finally { file.delete() }
            }
            "start" -> ConnectionService.start(context)
            "stop-action" -> context.startService(Intent(context, ConnectionService::class.java).setAction(ConnectionService.ACTION_STOP))
            "configure" -> {
                val settings = AgentSettings(context)
                settings.endpoint = intent.getStringExtra("endpoint") ?: return
                settings.token = intent.getStringExtra("token") ?: return
                settings.certificateSha256 = intent.getStringExtra("pin") ?: return
                settings.allowedPackages = setOf(context.packageName)
                settings.validate()
                ConnectionService.start(context)
            }
            "disconnect" -> ConnectionService.stop(context)
            "fixture" -> context.startActivity(Intent(context, DiagnosticsActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK).putExtra("sensitive", intent.getBooleanExtra("sensitive", false)))
            "post" -> TestNotifications.post(context)
            "cancel" -> TestNotifications.cancel(context)
            "clear" -> {
                ConnectionService.stop(context)
                context.getSharedPreferences("connection", Context.MODE_PRIVATE).edit().remove("token").remove("pin").apply()
            }
        }
    }
}
