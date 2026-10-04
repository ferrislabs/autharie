package fr.autharie.android.domain.usecase

import fr.autharie.android.domain.model.CreateDeploymentRequest
import fr.autharie.android.domain.model.Deployment
import fr.autharie.android.domain.repository.DeploymentRepository

class CreateDeploymentUseCase(
    private val repository: DeploymentRepository
) {
    suspend operator fun invoke(request: CreateDeploymentRequest): Deployment {
        return repository.createDeployment(request)
    }
}
