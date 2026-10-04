package fr.autharie.android.domain.usecase

import fr.autharie.android.domain.repository.AuthRepository
import fr.autharie.android.domain.model.AuthToken

class DirectLoginUseCase(
    private val repository: AuthRepository
) {
    suspend operator fun invoke(
        username: String,
        password: String
    ): Result<AuthToken> = repository.loginWithPassword(username, password)
}
