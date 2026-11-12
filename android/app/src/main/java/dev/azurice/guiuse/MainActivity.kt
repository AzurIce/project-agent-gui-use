package dev.azurice.guiuse

import android.app.Activity
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.text.InputType
import android.widget.*

/** The activity is a control panel; closing it never stops the foreground service. */
class MainActivity : Activity() {
    private lateinit var status: TextView
    private lateinit var toggle: Button
    private val handler = Handler(Looper.getMainLooper())
    private val refresh = object : Runnable { override fun run() { renderStatus(); handler.postDelayed(this, 1000) } }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val settings = AgentSettings(this)
        val column = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(32, 48, 32, 48) }
        column.addView(TextView(this).apply { text = "GUI Use"; textSize = 26f })
        status = TextView(this).also { it.setPadding(0, 24, 0, 24); column.addView(it) }
        fun button(label: String, action: () -> Unit): Button = Button(this).apply {
            text = label; setOnClickListener { action() }; column.addView(this)
        }
        toggle = button("启动服务") {
            if (settings.serviceEnabled) ConnectionService.stop(this)
            else {
                if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                    requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), 1)
                }
                ConnectionService.start(this)
            }
            renderStatus()
        }
        column.addView(TextView(this).apply {
            text = "USB 连接电脑后，直接运行 agent 命令即可自动配置连接。\n服务启动后可以退出此页面；电脑不在线时会等待并自动重连。"
        })
        button("无障碍权限") { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }
        button("通知访问权限") { startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)) }
        button("后台与电池设置") { startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)) }
        button("打开本地测试页") { startActivity(Intent(this, DiagnosticsActivity::class.java)) }
        val advanced = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; visibility = android.view.View.GONE }
        val advancedToggle = button("高级连接设置") { }
        column.addView(advanced)
        fun field(label: String, value: String, password: Boolean = false): EditText {
            advanced.addView(TextView(this).apply { text = label })
            return EditText(this).apply {
                setText(value); setSingleLine(true)
                inputType = if (password) InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD else InputType.TYPE_CLASS_TEXT
                advanced.addView(this)
            }
        }
        val endpoint = field("Host WebSocket", settings.endpoint)
        val token = field("Pairing token", settings.token, true)
        val pin = field("Certificate SHA-256", settings.certificateSha256)
        val allowed = field("Allowed packages", settings.allowedPackages.joinToString(","))
        advancedToggle.setOnClickListener {
            if (advanced.visibility == android.view.View.GONE) {
                endpoint.setText(settings.endpoint); token.setText(settings.token)
                pin.setText(settings.certificateSha256); allowed.setText(settings.allowedPackages.joinToString(","))
                advanced.visibility = android.view.View.VISIBLE
            } else advanced.visibility = android.view.View.GONE
        }
        advanced.addView(Button(this).apply { text = "保存高级设置"; setOnClickListener {
            settings.endpoint = endpoint.text.toString(); settings.token = token.text.toString(); settings.certificateSha256 = pin.text.toString()
            settings.allowedPackages = allowed.text.toString().split(',').map(String::trim).filter(String::isNotEmpty).toSet()
            try {
                settings.validate()
                if (settings.serviceEnabled) ConnectionService.start(this@MainActivity)
                Toast.makeText(this@MainActivity, "已保存", Toast.LENGTH_SHORT).show()
            } catch (failure: IllegalArgumentException) { Toast.makeText(this@MainActivity, failure.message, Toast.LENGTH_LONG).show() }
        } })
        setContentView(ScrollView(this).apply { addView(column) })
    }

    override fun onResume() {
        super.onResume()
        if (AgentSettings(this).serviceEnabled && !AgentRuntime.serviceRunning) ConnectionService.start(this)
        handler.post(refresh)
    }
    override fun onPause() { handler.removeCallbacks(refresh); super.onPause() }
    private fun renderStatus() {
        if (!::status.isInitialized) return
        val settings = AgentSettings(this)
        val state = when {
            !settings.serviceEnabled -> "服务已停止"
            !AgentRuntime.serviceRunning -> "服务正在恢复"
            AgentRuntime.connectionStatus == "Connected" -> "服务运行中 · 已连接电脑"
            else -> "服务运行中 · 等待电脑"
        }
        status.text = "$state\n无障碍：${if (AgentRuntime.accessibility != null) "已启用" else "未启用"}    通知访问：${if (AgentRuntime.notifications != null) "已启用" else "未启用"}"
        toggle.text = if (settings.serviceEnabled) "停止服务" else "启动服务"
    }
}
