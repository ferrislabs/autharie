package fr.autharie.android.presentation.create

import fr.autharie.android.domain.model.CpuPreset
import fr.autharie.android.domain.model.Environment
import fr.autharie.android.domain.model.MemoryPreset
import fr.autharie.android.domain.model.Deployment

data class CreateDeploymentUiState(
    val step: CreateDeploymentStep = CreateDeploymentStep.EDIT,
    val name: String = "",
    val environment: Environment = Environment.STAGING,
    val replicas: Int = 2,
    val cpuPreset: CpuPreset = CpuPreset.MEDIUM,
    val memoryPreset: MemoryPreset = MemoryPreset.MEDIUM,
    val autoScalingEnabled: Boolean = true,
    val nameError: String? = null,
    val progressMessage: String = "Preparing…",
    val createdDeployment: Deployment? = null,
    val errorMessage: String? = null
)
