@file:Suppress("DEPRECATION")

package dev.azurice.guiuse

import android.app.*
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.widget.*

/** A local-only surface; test actions and replies never leave this application. */
class DiagnosticsActivity : Activity() {
    private val handler = Handler(Looper.getMainLooper())
    private lateinit var reply: TextView
    private val update = object : Runnable {
        override fun run() {
            val value = getSharedPreferences("diagnostics", MODE_PRIVATE).getString("reply", "none")
            val text = "Reply: $value"
            if (reply.text.toString() != text) reply.text = text
            handler.postDelayed(this, 250)
        }
    }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val content = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(32, 48, 32, 32) }
        content.isFocusableInTouchMode = true
        content.requestFocus()
        content.addView(TextView(this).apply { text = "GUI Use local test surface"; textSize = 22f })
        content.addView(EditText(this).apply { id = R.id.test_input; setText("Initial text"); hint = "Test input"; setSingleLine(true) })
        val counter = TextView(this).apply { id = R.id.test_counter; text = "Count: 0" }
        content.addView(counter)
        var count = 0
        content.addView(Button(this).apply { id = R.id.test_increment; text = "Increment"; setOnClickListener { count += 1; counter.text = "Count: $count" } })
        content.addView(CheckBox(this).apply { id = R.id.test_checkbox; text = "Local toggle" })
        reply = TextView(this).also { it.id = R.id.test_reply; content.addView(it) }
        content.addView(Button(this).apply { text = "Post local test notification"; setOnClickListener { TestNotifications.post(this@DiagnosticsActivity) } })
        if (intent.getBooleanExtra("sensitive", false)) content.addView(EditText(this).apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD; setText("fake-local-value")
        })
        repeat(40) { index -> content.addView(TextView(this).apply { text = "Test row $index"; setPadding(0, 16, 0, 16) }) }
        setContentView(ScrollView(this).apply { id = R.id.test_scroll; addView(content) })
    }
    override fun onResume() { super.onResume(); handler.post(update) }
    override fun onPause() { handler.removeCallbacks(update); super.onPause() }
}

object TestNotifications {
    const val ID = 42
    fun post(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("gui-use-test", "Local automation tests", NotificationManager.IMPORTANCE_DEFAULT))
        val mutable = if (android.os.Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0
        val reply = PendingIntent.getBroadcast(context, 42, Intent(context, TestReplyReceiver::class.java), PendingIntent.FLAG_UPDATE_CURRENT or mutable)
        val input = RemoteInput.Builder("local_reply").setLabel("Local reply").build()
        val action = Notification.Action.Builder(android.R.drawable.ic_menu_send, "Reply locally", reply).addRemoteInput(input).build()
        manager.notify(ID, Notification.Builder(context, "gui-use-test").setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("GUI Use test notification").setContentText("Local fixture; no external messages are sent.").addAction(action).build())
    }
    fun cancel(context: Context) { context.getSystemService(NotificationManager::class.java).cancel(ID) }
}

class TestReplyReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val reply = RemoteInput.getResultsFromIntent(intent)?.getCharSequence("local_reply") ?: return
        context.getSharedPreferences("diagnostics", Context.MODE_PRIVATE).edit().putString("reply", reply.toString()).apply()
    }
}
