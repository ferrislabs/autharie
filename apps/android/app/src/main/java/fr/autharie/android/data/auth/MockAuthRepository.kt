package fr.autharie.android.data.auth

import fr.autharie.android.domain.model.AuthRequest
import fr.autharie.android.domain.model.AuthToken
import fr.autharie.android.domain.repository.AuthRepository
import javax.inject.Inject
import kotlinx.coroutines.delay

class MockAuthRepository @Inject constructor(
    private val authSession: AuthSession
) : AuthRepository {
    override suspend fun createAuthorizationRequest(): Result<AuthRequest> {
        // Mocked authentication for local development.
        delay(500)
        return Result.success(
            AuthRequest(
                authorizationUrl = "https://example.com/auth",
                state = "mock-state"
            )
        )
    }

    override suspend fun exchangeToken(
        authorizationCode: String,
        state: String
    ): Result<AuthToken> {
        // Mocked token exchange for local development.
        delay(500)
        return Result.success(
            AuthToken(
                accessToken = "fake_access_token",
                expiresIn = 3600,
                refreshToken = "fake_refresh_token",
                idToken = "fake_id_token"
            ).also { token -> authSession.setToken(token) }
        )
    }

    override suspend fun loginWithPassword(
        username: String,
        password: String
    ): Result<AuthToken> {
        delay(500)
        return Result.success(
            AuthToken(
                accessToken = "fake_access_token",
                expiresIn = 3600,
                refreshToken = "fake_refresh_token",
                idToken = "fake_id_token"
            ).also { token -> authSession.setToken(token) }
        )
    }

    override suspend fun logout(): Result<Unit> {
        authSession.clear()
        return Result.success(Unit)
    }
}
