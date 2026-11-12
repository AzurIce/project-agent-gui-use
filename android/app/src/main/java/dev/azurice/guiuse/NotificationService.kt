package dev.azurice.guiuse

import android.app.Notification
import android.app.RemoteInput
import android.content.Intent
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import dev.azurice.guiuse.protocol.NotificationAction
import dev.azurice.guiuse.protocol.NotificationInfo
import dev.azurice.guiuse.protocol.NotificationPostedData
import dev.azurice.guiuse.protocol.NotificationRemovedData
import dev.azurice.guiuse.protocol.ProtocolEvents
import java.time.Instant

class NotificationService : NotificationListenerService() {
    override fun onListenerConnected() { AgentRuntime.notifications = this }
    override fun onListenerDisconnected() { if (AgentRuntime.notifications === this) AgentRuntime.notifications = null }
    override fun onDestroy() { onListenerDisconnected(); super.onDestroy() }

    private fun visible(sbn: StatusBarNotification): Boolean =
        sbn.notification.channelId != ConnectionService.CHANNEL &&
            runCatching { SafetyPolicy(this).requirePackage(sbn.packageName) }.isSuccess

    fun list(limit: Int): List<NotificationInfo> = (activeNotifications ?: emptyArray())
        .filter(::visible).sortedByDescending { it.postTime }.take(limit).map(::info)

    private fun info(sbn: StatusBarNotification): NotificationInfo {
        val notification = sbn.notification
        return NotificationInfo(sbn.key, sbn.packageName,
            notification.extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()?.take(16384),
            (notification.extras.getCharSequence(Notification.EXTRA_BIG_TEXT)
                ?: notification.extras.getCharSequence(Notification.EXTRA_TEXT))?.toString()?.take(16384),
            Instant.ofEpochMilli(sbn.postTime).toString(), sbn.isOngoing,
            (notification.actions ?: emptyArray()).mapIndexed { index, action ->
                NotificationAction(index, action.title?.toString() ?: "", action.remoteInputs?.any { it.allowFreeFormInput } == true)
            })
    }

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        if (sbn == null || !visible(sbn)) return
        val data = info(sbn)
        AgentRuntime.emit("notification.posted") { seq, ts ->
            ProtocolEvents.notificationPosted(seq, ts, NotificationPostedData(data.key, data.package_name, data.title, data.text, data.posted_at), AgentRuntime.json)
        }
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification?) {
        if (sbn == null || !visible(sbn)) return
        AgentRuntime.emit("notification.removed") { seq, ts ->
            ProtocolEvents.notificationRemoved(seq, ts, NotificationRemovedData(sbn.key), AgentRuntime.json)
        }
    }

    fun reply(key: String, text: String, actionIndex: Int?) {
        if (text.isEmpty() || text.length > 10000) throw DeviceRpcException(-32602, "Invalid reply length")
        val notification = activeNotifications?.firstOrNull { it.key == key }
            ?: throw DeviceRpcException(2001, "Notification is no longer active")
        SafetyPolicy(this).requirePackage(notification.packageName)
        val actions = notification.notification.actions ?: emptyArray()
        val index = actionIndex ?: actions.indexOfFirst { action -> action.remoteInputs?.any { it.allowFreeFormInput } == true }
        val action = actions.getOrNull(index) ?: throw DeviceRpcException(2002, "No matching reply action")
        val inputs = action.remoteInputs?.filter { it.allowFreeFormInput }?.toTypedArray()
            ?.takeIf { it.isNotEmpty() } ?: throw DeviceRpcException(2002, "Action has no free-form RemoteInput")
        if (inputs.size != 1) throw DeviceRpcException(2002, "Multiple reply fields are ambiguous")
        val intent = Intent()
        RemoteInput.addResultsToIntent(inputs, intent, Bundle().apply { inputs.forEach { putCharSequence(it.resultKey, text) } })
        RemoteInput.setResultsSource(intent, RemoteInput.SOURCE_FREE_FORM_INPUT)
        try { action.actionIntent.send(this, 0, intent) }
        catch (_: android.app.PendingIntent.CanceledException) { throw DeviceRpcException(2001, "Reply action expired") }
    }
}
