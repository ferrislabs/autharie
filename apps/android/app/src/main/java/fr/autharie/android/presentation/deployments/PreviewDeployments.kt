package fr.autharie.android.presentation.deployments

import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.runtime.Composable
import fr.autharie.android.domain.model.DeploymentStatus
import fr.autharie.android.domain.model.IamProvider
import fr.autharie.android.ui.theme.AndroidTheme

private val previewDeployments = listOf(
    DeploymentUiModel(
        id = "dep-1",
        name = "Keycloak - Core",
        environment = "Production",
        status = DeploymentStatus.RUNNING,
        provider = IamProvider.KEYCLOAK,
        cluster = "iam-prod-01",
        namespace = "keycloak",
        version = "24.0.2",
        endpoint = "https://iam.autharie.io",
        region = "eu-west-1",
        updatedAt = "2024-08-12 10:24"
    ),
    DeploymentUiModel(
        id = "dep-2",
        name = "Ferriskey - Edge",
        environment = "Staging",
        status = DeploymentStatus.DEPLOYING,
        provider = IamProvider.FERRISKEY,
        cluster = "iam-stg-02",
        namespace = "ferriskey",
        version = "2.3.1",
        endpoint = "https://iam-stg.autharie.io",
        region = "eu-west-1",
        updatedAt = "2024-08-11 18:03"
    ),
    DeploymentUiModel(
        id = "dep-3",
        name = "Keycloak - IAM Portal",
        environment = "Development",
        status = DeploymentStatus.FAILED,
        provider = IamProvider.KEYCLOAK,
        cluster = "iam-dev-02",
        namespace = "keycloak",
        version = "24.0.1",
        endpoint = "https://iam-dev.autharie.io",
        region = "us-east-1",
        updatedAt = "2024-08-12 08:41"
    )
)

@Preview(showBackground = true)
@Composable
private fun DeploymentsSuccessPreview() {
    AndroidTheme {
        DeploymentsScreen(
            uiState = DeploymentsUiState.Success(
                deployments = previewDeployments,
                isRefreshing = false
            ),
            onRefresh = {},
            onDeploymentClick = {},
            onCreateDeployment = {}
        )
    }
}

@Preview(showBackground = true)
@Composable
private fun DeploymentsLoadingPreview() {
    AndroidTheme {
        DeploymentsScreen(
            uiState = DeploymentsUiState.Loading,
            onRefresh = {},
            onDeploymentClick = {},
            onCreateDeployment = {}
        )
    }
}

@Preview(showBackground = true)
@Composable
private fun DeploymentsEmptyPreview() {
    AndroidTheme {
        DeploymentsScreen(
            uiState = DeploymentsUiState.Success(
                deployments = emptyList(),
                isRefreshing = false
            ),
            onRefresh = {},
            onDeploymentClick = {},
            onCreateDeployment = {}
        )
    }
}

@Preview(showBackground = true)
@Composable
private fun DeploymentsErrorPreview() {
    AndroidTheme {
        DeploymentsScreen(
            uiState = DeploymentsUiState.Error("Unable to load deployments."),
            onRefresh = {},
            onDeploymentClick = {},
            onCreateDeployment = {}
        )
    }
}
