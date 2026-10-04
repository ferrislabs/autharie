package fr.autharie.android.domain.repository

import fr.autharie.android.domain.model.Deployment
import fr.autharie.android.domain.model.CreateDeploymentRequest

interface DeploymentRepository {
    suspend fun getDeployments(): List<Deployment>
    suspend fun createDeployment(request: CreateDeploymentRequest): Deployment
    suspend fun deleteDeployment(id: String)
}
