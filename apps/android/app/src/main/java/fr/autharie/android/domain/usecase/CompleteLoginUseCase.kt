package fr.autharie.android.domain.usecase

import fr.autharie.android.domain.model.AuthToken
import fr.autharie.android.domain.repository.AuthRepository

class CompleteLoginUseCase(
    private val repository: AuthRepository
) {
    suspend operator fun invoke(
        authorizationCode: String,
        state: String
    ): Result<AuthToken> = repository.exchangeToken(authorizationCode, state)
}
