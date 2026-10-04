use autharie_domain::{
    CoreError,
    user::{User, commands::CreateUserCommand, ports::UserService, service::UserServiceImpl},
};
use autharie_macros::transactional;

use crate::AutharieService;

impl UserService for AutharieService {
    #[transactional(user)]
    async fn create_user(&self, command: CreateUserCommand) -> Result<User, CoreError> {
        UserServiceImpl::new(user_repository)
            .create_user(command)
            .await
    }
}
