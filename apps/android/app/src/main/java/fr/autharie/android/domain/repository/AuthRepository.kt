package fr.autharie.android.domain.repository

import fr.autharie.android.domain.model.AuthToken
import fr.autharie.android.domain.model.AuthRequest

interface AuthRepository {
    suspend fun createAuthorizationRequest(): Result<AuthRequest>
    suspend fun exchangeToken(authorizationCode: String, state: String): Result<AuthToken>
    suspend fun loginWithPassword(username: String, password: String): Result<AuthToken>
    suspend fun logout(): Result<Unit>
}
