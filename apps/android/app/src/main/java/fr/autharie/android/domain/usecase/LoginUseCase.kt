package fr.autharie.android.domain.usecase

import fr.autharie.android.domain.model.AuthRequest
import fr.autharie.android.domain.repository.AuthRepository

class LoginUseCase(
    private val repository: AuthRepository
) {
    suspend operator fun invoke(): Result<AuthRequest> = repository.createAuthorizationRequest()
}
