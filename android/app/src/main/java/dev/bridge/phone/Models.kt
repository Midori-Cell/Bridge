package dev.bridge.phone

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/** A PC this phone is paired with. Its certificate fingerprint is its identity. */
@Serializable
data class Pc(
    val name: String,
    val host: String,
    val port: Int,
    val fingerprint: String,
    val token: String,
) {
    val id: String get() = fingerprint
}

@Serializable
data class Reply(val mid: String = "", val src: String = "", val preview: String = "")

@Serializable
data class FileRef(
    val id: String,
    val name: String,
    val size: Long = 0,
    val sha256: String = "",
    /** "to_phone" (the PC sent it) or "to_pc" (this phone sent it). */
    val dir: String = "",
)

/** One entry of the shared history: a text note or a file. `src` is "pc" or the name of the phone that wrote it. */
@Serializable
data class Note(
    val id: Long,
    val mid: String = "",
    val ts: Double = 0.0,
    val src: String = "",
    val kind: String = "text",
    val text: String? = null,
    val file: FileRef? = null,
    val reply: Reply? = null,
    /** Written on this phone but not delivered yet (the PC was not reachable); it is sent as soon as the PC is back. */
    val pending: Boolean = false,
) {
    val fromPc: Boolean get() = src == "pc"
    val isFile: Boolean get() = kind == "file" && file != null
}

/** A text written while the PC was not reachable. It is saved and sent as soon as the PC is online. */
@Serializable
data class Queued(
    val id: String,
    val pcId: String,
    val text: String,
    val replyTo: String? = null,
    val ts: Double,
)

@Serializable
data class NotesResponse(
    val notes: List<Note> = emptyList(),
    @SerialName("last_id") val lastId: Long = 0,
    val deleted: List<String> = emptyList(),
    val del: Long = 0,
)

@Serializable
data class NoteResponse(val note: Note)

@Serializable
data class PairResponse(
    val token: String,
    @SerialName("device_id") val deviceId: String = "",
    val fingerprint: String,
    val pc: String = "",
)

@Serializable
data class PingResponse(val device: String = "", val pc: String = "")

@Serializable
data class CreateResponse(val id: String, val state: String = "", val done: Boolean = false)

@Serializable
data class StatusResponse(val state: String = "", val received: Long = 0)

@Serializable
data class PutResponse(val received: Long = 0, val done: Boolean = false)

enum class Link { Connecting, Online, Offline }

enum class TState { Waiting, Queued, Active, Done, Failed }

/** A file on its way, in either direction. For a download `key` is the file id. */
data class Transfer(
    val key: String,
    val pcId: String,
    val name: String,
    val size: Long,
    val done: Long = 0,
    val upload: Boolean,
    val state: TState = TState.Waiting,
    val error: String? = null,
) {
    val running: Boolean get() = state == TState.Waiting || state == TState.Queued || state == TState.Active
}
