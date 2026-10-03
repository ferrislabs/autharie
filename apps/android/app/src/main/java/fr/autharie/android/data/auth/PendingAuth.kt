package fr.autharie.android.data.auth

data class PendingAuth(
    val codeVerifier: String,
    val state: String,
    val redirectUri: String
)
