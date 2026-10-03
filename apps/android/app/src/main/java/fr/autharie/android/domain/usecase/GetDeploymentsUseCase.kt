package fr.autharie.android.domain.usecase

import fr.autharie.android.domain.model.Deployment
import fr.autharie.android.domain.repository.DeploymentRepository

class GetDeploymentsUseCase(
    private val repository: DeploymentRepository
) {
    suspend operator fun invoke(): List<Deployment> = repository.getDeployments()
}
