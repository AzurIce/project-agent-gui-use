package dev.azurice.guiuse

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.IBinder
import android.net.ConnectivityManager
import android.net.Network
import dev.azurice.guiuse.protocol.DeviceInfo
import dev.azurice.guiuse.protocol.HelloParams
import dev.azurice.guiuse.protocol.HelloResult
import dev.azurice.guiuse.protocol.Methods
import dev.azurice.guiuse.protocol.ProtocolDispatch
import java.nio.ByteBuffer
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.cert.X509Certificate
import java.time.Instant
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager
import kotlinx.coroutines.*
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.*
import okhttp3.*
import okio.ByteString.Companion.toByteString

class ConnectionService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var connection: Job? = null
    @Volatile private var activeSession: Session? = null
    private val retry = kotlinx.coroutines.channels.Channel<Unit>(kotlinx.coroutines.channels.Channel.CONFLATED)
    private val network = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) { activeSession?.cancel(); retry.trySend(Unit) }
        override fun onLost(network: Network) { activeSession?.cancel(); retry.trySend(Unit) }
    }
    private val screen = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            AgentRuntime.screenState(context)
            if (AgentRuntime.connectionStatus != "Connected") retry.trySend(Unit)
        }
    }

    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel(CHANNEL, "Automation connection", NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        startForeground(1, Notification.Builder(this, CHANNEL).setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentTitle("GUI Use session").setContentText("Remote automation is enabled").setContentIntent(open).setOngoing(true).build())
        val filter = IntentFilter().apply { addAction(Intent.ACTION_SCREEN_ON); addAction(Intent.ACTION_SCREEN_OFF); addAction(Intent.ACTION_USER_PRESENT); addAction(Intent.ACTION_POWER_CONNECTED); addAction(Intent.ACTION_POWER_DISCONNECTED) }
        if (Build.VERSION.SDK_INT >= 33) registerReceiver(screen, filter, RECEIVER_NOT_EXPORTED) else registerReceiver(screen, filter)
        getSystemService(ConnectivityManager::class.java).registerDefaultNetworkCallback(network)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (connection?.isActive == true) return START_NOT_STICKY
        connection = scope.launch {
            val settings = AgentSettings(this@ConnectionService)
            try { settings.validate() } catch (_: IllegalArgumentException) { AgentRuntime.connectionStatus = "Invalid pairing configuration"; stopSelf(); return@launch }
            var backoff = 1000L
            while (isActive) {
                AgentRuntime.connectionStatus = "Connecting"
                val started = android.os.SystemClock.uptimeMillis()
                try { Session(this@ConnectionService, settings).also { activeSession = it }.run() }
                catch (error: CancellationException) { throw error }
                catch (_: Exception) { /* No token, notification text or TLS material in logs. */ }
                finally { activeSession = null }
                AgentRuntime.connectionStatus = "Disconnected; reconnecting"
                if (android.os.SystemClock.uptimeMillis() - started > 30000) backoff = 1000
                withTimeoutOrNull(backoff) { retry.receive() }
                backoff = (backoff * 2).coerceAtMost(30000)
            }
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        scope.cancel(); AgentRuntime.disconnected(); AgentRuntime.connectionStatus = "Disconnected"
        unregisterReceiver(screen)
        getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(network)
        super.onDestroy()
    }
    override fun onBind(intent: Intent?): IBinder? = null
    companion object { const val CHANNEL = "gui-use-connection" }
}

private class Session(private val context: Context, private val settings: AgentSettings) {
    private val json = AgentRuntime.json
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val ready = CompletableDeferred<Unit>()
    private val closed = CompletableDeferred<Unit>()
    private val jobs = ConcurrentHashMap<Long, Job>()
    private val outstanding = AtomicInteger()
    private val device = AndroidDevice(context)
    @Volatile private var socket: WebSocket? = null
    @Volatile private var authenticated = false

    fun cancel() { socket?.cancel() }

    // A paired self-signed leaf is authenticated by its exact SHA-256 digest and
    // validity period, not by the public CA store. This is not a trust-all manager.
    @android.annotation.SuppressLint("CustomX509TrustManager")
    private fun client(): OkHttpClient {
        val expected = settings.certificateSha256.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
        val trust = object : X509TrustManager {
            override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
            override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) = throw java.security.cert.CertificateException("Client certificates unsupported")
            override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
                val leaf = chain.firstOrNull() ?: throw java.security.cert.CertificateException("Missing server certificate")
                leaf.checkValidity()
                if (!MessageDigest.isEqual(expected, MessageDigest.getInstance("SHA-256").digest(leaf.encoded))) throw java.security.cert.CertificateException("Certificate fingerprint mismatch")
            }
        }
        val tls = SSLContext.getInstance("TLS").apply { init(null, arrayOf(trust), SecureRandom()) }
        return OkHttpClient.Builder().sslSocketFactory(tls.socketFactory, trust)
            // An explicitly paired certificate authenticates the host independent of LAN/DNS names.
            .hostnameVerifier { _, session ->
                runCatching { MessageDigest.isEqual(expected, MessageDigest.getInstance("SHA-256").digest(session.peerCertificates.first().encoded)) }.getOrDefault(false)
            }.pingInterval(15, TimeUnit.SECONDS).connectTimeout(10, TimeUnit.SECONDS).readTimeout(0, TimeUnit.SECONDS).build()
    }

    suspend fun run() {
        val client = client()
        try {
            socket = client.newWebSocket(Request.Builder().url(settings.endpoint).build(), object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) {
                    socket = webSocket
                    val hello = HelloParams(1, settings.token, DeviceInfo(settings.deviceId, Build.MODEL, Build.VERSION.SDK_INT), Methods.ALL.toList())
                    webSocket.send(buildJsonObject { put("jsonrpc", "2.0"); put("id", 0); put("method", "hello"); put("params", json.encodeToJsonElement(hello)) }.toString())
                }
                override fun onMessage(webSocket: WebSocket, text: String) {
                    if (text.length > 1024 * 1024) { webSocket.close(1009, "Message too large"); return }
                    val message = try { json.parseToJsonElement(text).jsonObject } catch (_: Exception) { error(JsonNull, -32700, "Invalid JSON"); return }
                    if (!authenticated) {
                        try {
                            require(message["jsonrpc"]?.jsonPrimitive?.content == "2.0" && message["id"]?.jsonPrimitive?.longOrNull == 0L && message["error"] == null)
                            val hello = json.decodeFromJsonElement<HelloResult>(message.getValue("result"))
                            require(hello.protocol == 1)
                            AgentRuntime.connected(hello.events.toSet()) { event ->
                                if (webSocket.queueSize() > 1024 * 1024) { webSocket.close(1011, "Event queue overflow"); false } else webSocket.send(event.toString())
                            }
                            AgentRuntime.connectionStatus = "Connected"
                            authenticated = true
                            ready.complete(Unit)
                            AgentRuntime.screenState(context)
                        } catch (_: Exception) { ready.completeExceptionally(IllegalStateException("Handshake rejected")); webSocket.close(1008, "Handshake rejected") }
                        return
                    }
                    handle(message)
                }
                override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) { ready.completeExceptionally(t); closed.complete(Unit) }
                override fun onClosed(webSocket: WebSocket, code: Int, reason: String) { closed.complete(Unit) }
                override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(code, null) }
            })
            withTimeout(10000) { ready.await() }
            closed.await()
        } finally {
            AgentRuntime.disconnected(); scope.cancel(); device.clearNodes(); socket?.cancel()
            client.dispatcher.executorService.shutdown(); client.connectionPool.evictAll()
        }
    }

    private fun handle(message: JsonObject) {
        val id = message["id"]
        val method = (message["method"] as? JsonPrimitive)?.content
        if (method == "$/cancel" && id == null) {
            val cancelled = (message["params"] as? JsonObject)?.get("id") as? JsonPrimitive
            cancelled?.longOrNull?.let { jobs[it]?.cancel() }; return
        }
        val number = (id as? JsonPrimitive)?.takeUnless { it.isString }?.longOrNull
        if (message["jsonrpc"] != JsonPrimitive("2.0") || method == null || number == null || number !in 0..4294967295L) {
            error(JsonNull, -32600, "Invalid JSON-RPC request"); return
        }
        if (method !in Methods.ALL) { error(id, -32601, "Method not found"); return }
        val params = message["params"] ?: JsonObject(emptyMap())
        if (params !is JsonObject) { error(id, -32602, "Parameters must be an object"); return }
        if (outstanding.incrementAndGet() > 32) { outstanding.decrementAndGet(); error(id, -32603, "Too many outstanding requests"); return }
        val job = scope.launch(start = CoroutineStart.LAZY) {
            var outcome = "ok"
            try {
                val frame = RequestFrame()
                val result = withTimeout(65000) { withContext(frame) { ProtocolDispatch.dispatch(method, params, json, device) } }
                val response = buildJsonObject { put("jsonrpc", "2.0"); put("id", id); put("result", result) }.toString()
                if (response.toByteArray().size > 8 * 1024 * 1024) throw DeviceRpcException(2002, "Response exceeds transport limit")
                socket?.send(response)
                frame.binary?.let { bytes ->
                    val header = buildJsonObject { put("id", id); put("mime", "image/png") }.toString().toByteArray()
                    socket?.send(ByteBuffer.allocate(4 + header.size + bytes.size).putInt(header.size).put(header).put(bytes).array().toByteString())
                }
            } catch (failure: Exception) {
                val (code, description) = when (failure) {
                    is DeviceRpcException -> failure.code to failure.message
                    is SerializationException -> -32602 to "Invalid parameters"
                    is CancellationException -> 2004 to "Operation cancelled or timed out"
                    is SecurityException -> 2005 to "Android permission denied"
                    is IllegalArgumentException -> -32602 to "Invalid parameters"
                    else -> -32603 to "Device operation failed"
                }
                outcome = code.toString(); error(id, code, description)
            } finally {
                audit(method, outcome)
            }
        }
        if (jobs.putIfAbsent(number, job) != null) {
            outstanding.decrementAndGet(); error(id, -32600, "Duplicate request ID"); job.cancel()
        } else {
            job.invokeOnCompletion { jobs.remove(number, job); outstanding.decrementAndGet() }
            job.start()
        }
    }

    private fun error(id: JsonElement?, code: Int, message: String) {
        socket?.send(buildJsonObject { put("jsonrpc", "2.0"); put("id", id ?: JsonNull); putJsonObject("error") { put("code", code); put("message", message) } }.toString())
    }

    private fun audit(method: String, outcome: String) {
        runCatching {
            val file = java.io.File(context.filesDir, "audit.jsonl")
            if (file.length() > 262144) { val previous = java.io.File(context.filesDir, "audit.previous.jsonl"); previous.delete(); file.renameTo(previous) }
            file.appendText(buildJsonObject { put("ts", Instant.now().toString()); put("method", method); put("outcome", outcome) }.toString() + "\n")
        }
    }
}
