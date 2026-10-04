Feature: A deployment runs on a cluster created in the customer's own cloud account

  A customer registers a cloud credential and creates a deployment on their
  cloud. The control plane records the intent, a worker builds the cluster with
  the customer's credential, bootstraps it, and the cluster's first heartbeat
  makes it active. Provisioning is asynchronous, so its outcome is read on the
  data plane and not on the create call.

  @spec-ccp-1
  Scenario: a customer registers a credential with the required permissions
    Given a provider whose verification accepts the credential
    When the customer registers the credential labelled "production"
    Then the credential is registered for the organisation with the label "production"
    And the credential is listed for the organisation
    And exactly one sealed secret is stored and it opens back to the secret the customer sent

  @spec-ccp-2
  Scenario: a credential with missing or excess permissions is refused with the list
    Given a provider reporting the missing permission "KubernetesFullAccess"
    When the customer registers the credential labelled "production"
    Then the registration is refused for the missing permissions "KubernetesFullAccess"
    When the provider now reports the excess permission "IAMManager" and the customer registers again
    Then the registration is refused for the excess permissions "IAMManager"
    And nothing was stored for either attempt

  @spec-ccp-3
  Scenario: a credential is never returned by the API, logged, or put in an action payload
    Given a provider whose verification accepts the credential
    When the customer registers the credential labelled "production"
    And the customer lists the credentials
    And the customer creates a customer cloud deployment
    And the worker provisions the cluster on a healthy Scaleway
    And the provider now rejects the credential as invalid and the customer registers again
    Then the plaintext secret is in no response the API would return
    And the plaintext secret is in no captured log line
    And the plaintext secret is in no stored row or ciphertext
    And the plaintext secret is on no helm command line and in no helm values file

  @spec-ccp-4
  Scenario: a credential used by a deployment cannot be deleted
    Given a provider whose verification accepts the credential
    And a registered credential
    And a customer cloud deployment on that credential
    When the customer deletes the credential
    Then the deletion is refused because the credential is in use
    And the credential and its sealed secret are still stored
    And a second credential used by nothing can be deleted

  @spec-ccp-5
  Scenario: a `dev` profile with two nodes is refused
    Given a registered credential
    When the customer asks for a "dev" profile on control plane "kapsule" and node type "PRO2-S" with 2 to 2 nodes and 1 replicas
    Then the profile is refused as "AboveModeCeiling"
    And the refusal reads "dev allows at most 1 nodes, not 2"
    And nothing was created

  @spec-ccp-6
  Scenario: a `ha` profile with two minimum nodes is refused
    Given a registered credential
    When the customer asks for a "ha" profile on control plane "kapsule" and node type "PRO2-S" with 2 to 5 nodes and 2 replicas
    Then the profile is refused as "BelowModeFloor"
    And the refusal reads "ha needs at least 3 nodes, not 2"
    And nothing was created

  @spec-ccp-7
  Scenario: a node type absent from the region catalog is refused
    Given a registered credential
    When the customer asks for a "standard" profile on control plane "kapsule" and node type "GIGANTIC-XL" with 2 to 4 nodes and 2 replicas
    Then the profile is refused as "NodeTypeUnavailable"
    And the refusal reads "node type 'GIGANTIC-XL' is not offered in this region"
    And nothing was created

  @spec-ccp-8
  Scenario: the estimated monthly cost is shown for a valid profile before creation
    Given a registered credential
    When the customer asks the estimate of a "ha" profile on control plane "kapsule-dedicated-4" and node type "PRO2-S" with 3 to 10 nodes and 2 replicas
    Then the estimate runs from 32400 to 82800 minor units a month
    And nothing was created
    When the customer asks the estimate of a "dev" profile on control plane "kapsule" and node type "PRO2-S" with 2 to 2 nodes and 1 replicas
    Then no estimate is given and the profile is refused as "AboveModeCeiling"

  @spec-ccp-9
  Scenario: a non-FerrisKey deployment cannot use `CustomerCloud`
    Given a registered credential
    When the customer builds a creation request of kind "keycloak" on the customer cloud
    Then the request is refused because a keycloak deployment cannot run in the customer's cloud
    And no deployment and no data plane were recorded
    And a creation request of kind "ferriskey" on the customer cloud is accepted

  @spec-ccp-10
  Scenario: a `CustomerCloud` deployment creates one cluster, bootstraps it, registers the data plane and reaches `Active`
    Given a registered credential
    When the customer creates a customer cloud deployment
    Then the deployment is "pending" and its data plane is provisioning without a Herald binding
    And the create request did not provision anything
    When the worker provisions the cluster on a healthy Scaleway
    Then Scaleway was asked for exactly one private network, one cluster and one node pool
    And the data plane carries the Herald binding minted by the bootstrap and the capacity of the nodes
    And the data plane is still provisioning and its provisioning status is "provisioning"
    And the chart prerequisites and then the data plane chart were installed once
    When the worker runs a second time on a healthy Scaleway
    Then Scaleway was asked for nothing more and the report says nothing was provisioned
    When the first heartbeat of the data plane arrives
    Then the data plane is active and its provisioning status is "ready"

  @spec-ccp-11
  Scenario: a quota failure ends as a `Failed` data plane with a readable reason
    Given a registered credential
    When the customer creates a customer cloud deployment
    And the worker provisions the cluster on a Scaleway that refuses the cluster for quota
    Then the worker reports one failed provisioning
    And the data plane is failed with the reason "the provider quota in this account does not allow this cluster"
    And the provisioning status is "failed" with that reason
    And everything the failed attempt created was released
    And nothing was bootstrapped

  @spec-ccp-12
  Scenario: the kubeconfig is discarded after bootstrap
    Given a registered credential
    And a customer cloud deployment on that credential
    When the worker provisions the cluster on a healthy Scaleway
    Then the kubeconfig the chart installs saw is the one Scaleway served
    And the kubeconfig files no longer exist once the bootstrap returned
    And the kubeconfig is on no helm command line and in no values file
    And the kubeconfig is in no log line and in no stored state

  @spec-ccp-13
  Scenario: deleting the deployment deletes the cluster and the inventory shows nothing left
    Given a registered credential
    And a customer cloud deployment whose cluster is provisioned and active
    When the customer deletes the deployment
    And the worker tears the clusters down
    Then Scaleway received exactly one deletion of the pool, the cluster and the private network
    And the inventory shows nothing left
    And the Herald identity of the data plane was revoked once
    And the data plane is disabled and the deployment is "deleted"

  @spec-ccp-14
  Scenario: deprovision called twice succeeds and removes nothing the second time
    Given a registered credential
    And a customer cloud deployment whose cluster is provisioned and active
    When the customer deletes the deployment
    And the worker tears the clusters down
    And the worker tears the clusters down again
    Then the second pass removed and revoked nothing and left Scaleway untouched
    When the cluster is deprovisioned directly twice more
    Then both calls succeeded and Scaleway received no deletion
    And the inventory shows nothing left

  @spec-ccp-15
  Scenario: resizing `dev` to `standard` keeps the data plane and adds a node and a replica
    Given a registered credential
    And a customer cloud deployment on a "dev" cluster whose cluster is provisioned and active
    When the customer resizes the cluster to "standard" with 2 to 4 nodes and 2 replicas
    Then the resize is accepted with the profile "standard", 2 nodes at least and 2 replicas
    And Scaleway received exactly one node pool patch moving the pool to 2 nodes with autoscaling
    And the data plane is the same one, still active, with the same Herald binding
    And the persisted profile is "standard" with 2 to 4 nodes and 2 replicas
    And one audit entry records the change of profile

  @spec-ccp-16
  Scenario: resizing `ha` to `dev` is refused while replicas exceed the target
    Given a registered credential
    And a customer cloud deployment on a "ha" cluster whose cluster is provisioned and active
    When the customer resizes the cluster to "dev" with 1 to 1 nodes and 1 replicas
    Then the resize is refused with 2 replicas in use and 1 allowed
    And Scaleway received no node pool patch
    And the persisted profile is "ha" with 3 to 10 nodes and 2 replicas
    And no audit entry was recorded

  @spec-ccp-17
  Scenario: a control plane offer absent from the region catalog is refused
    Given a registered credential
    When the customer asks for a "standard" profile on control plane "kapsule-dedicated-64" and node type "PRO2-S" with 2 to 4 nodes and 2 replicas
    Then the profile is refused as "ControlPlaneUnavailable"
    And the refusal reads "control plane 'kapsule-dedicated-64' is not offered in this region"
    And nothing was created

  @spec-ccp-18
  Scenario: a `dev` profile with a dedicated control plane is refused
    Given a registered credential
    When the customer asks for a "dev" profile on control plane "kapsule-dedicated-4" and node type "PRO2-S" with 1 to 1 nodes and 1 replicas
    Then the profile is refused as "ControlPlaneNotAllowedForMode"
    And the refusal reads "dev does not accept the dedicated control plane 'kapsule-dedicated-4'"
    And nothing was created

  @spec-ccp-19
  Scenario: a `ha` profile with min 3 and max 10 nodes of a catalog instance type is accepted
    Given a registered credential
    When the customer asks for a "ha" profile on control plane "kapsule" and node type "PRO2-M" with 3 to 10 nodes and 2 replicas
    Then the profile is accepted for the credential with 3 to 10 nodes of "PRO2-M"
