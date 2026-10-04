package fr.autharie.android.domain.usecase

import fr.autharie.android.domain.repository.DeploymentRepository

class DeleteDeploymentUseCase(
    private val repository: DeploymentRepository
) {
    suspend operator fun invoke(id: String) {
        repository.deleteDeployment(id)
    }
}
