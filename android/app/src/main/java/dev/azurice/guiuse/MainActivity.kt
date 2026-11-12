package dev.azurice.guiuse

import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.text.InputType
import android.widget.*

class MainActivity : Activity() {
    private lateinit var status: TextView
    private val handler = Handler(Looper.getMainLooper())
    private val refresh = object : Runnable { override fun run() { renderStatus(); handler.postDelayed(this, 1000) } }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val settings = AgentSettings(this)
        val column = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(32, 48, 32, 48) }
        status = TextView(this).also { column.addView(it) }
        fun field(label: String, value: String, password: Boolean = false): EditText {
            column.addView(TextView(this).apply { text = label })
            return EditText(this).apply {
                setText(value); setSingleLine(true)
                inputType = if (password) InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD else InputType.TYPE_CLASS_TEXT
                column.addView(this)
            }
        }
        val endpoint = field("Host WebSocket (wss://)", settings.endpoint)
        val token = field("Pairing token", settings.token, true)
        val pin = field("Certificate SHA-256", settings.certificateSha256)
        val allowed = field("Allowed packages (comma separated)", settings.allowedPackages.joinToString(","))
        fun button(label: String, action: () -> Unit) { column.addView(Button(this).apply { text = label; setOnClickListener { action() } }) }
        button("Save and connect") {
            settings.endpoint = endpoint.text.toString(); settings.token = token.text.toString(); settings.certificateSha256 = pin.text.toString()
            settings.allowedPackages = allowed.text.toString().split(',').map(String::trim).filter(String::isNotEmpty).toSet()
            try {
                settings.validate(); stopService(Intent(this, ConnectionService::class.java)); startForegroundService(Intent(this, ConnectionService::class.java))
            } catch (failure: IllegalArgumentException) { Toast.makeText(this, failure.message, Toast.LENGTH_LONG).show() }
        }
        button("Disconnect") { stopService(Intent(this, ConnectionService::class.java)); renderStatus() }
        button("Accessibility settings") { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }
        button("Notification access settings") { startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)) }
        button("Open local test surface") { startActivity(Intent(this, DiagnosticsActivity::class.java)) }
        column.addView(TextView(this).apply { text = "Only allowlisted apps can be operated. Sensitive screens are blocked. Tokens are stored privately and never logged." })
        setContentView(ScrollView(this).apply { addView(column) })
    }

    override fun onResume() { super.onResume(); handler.post(refresh) }
    override fun onPause() { handler.removeCallbacks(refresh); super.onPause() }
    private fun renderStatus() {
        if (!::status.isInitialized) return
        fun enabled(key: String, service: Class<*>): Boolean {
            val expected = ComponentName(this, service).flattenToString()
            return (Settings.Secure.getString(contentResolver, key) ?: "").split(':').any { it.equals(expected, true) }
        }
        status.text = getString(R.string.connection_status, AgentRuntime.connectionStatus,
            enabled("enabled_accessibility_services", DeviceService::class.java).toString(),
            enabled("enabled_notification_listeners", NotificationService::class.java).toString())
    }
}
