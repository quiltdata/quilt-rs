use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;

use crate::local_domain;
use crate::uri::Namespace;
use crate::Error;
use crate::Res;

pub fn commit<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    domain: JString<'local>,
    namespace: JString<'local>,
    message: JString<'local>,
) -> jstring {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result: Res<String> = runtime.block_on(async {
        let domain: String = env.get_string(&domain)?.into();
        let namespace_str: String = env.get_string(&namespace)?.into();
        let namespace = Namespace::try_from(namespace_str)?;
        let message: String = env.get_string(&message)?.into();

        local_domain::commit_package(&domain.into(), namespace, message, None)
            .await?
            .map(|state| state.hash)
            .ok_or(Error::Commit("Nothing to commit".to_string()))
    });

    match result {
        Ok(commit_hash) => env
            .new_string(commit_hash)
            .expect("Couldn't create java string!")
            .into_raw(),
        Err(err) => panic!("{:?}", err),
    }
}

pub fn install<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    domain: JString<'local>,
    uri: JString<'local>,
) -> jstring {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result: Res<String> = runtime.block_on(async {
        let domain: String = env.get_string(&domain)?.into();
        let uri: String = env.get_string(&uri)?.into();

        let installed_package =
            local_domain::install_package_full(&domain.into(), &uri.parse()?, None).await?;
        let manifest_path = installed_package
            .manifest_path()
            .await?
            .display()
            .to_string();

        Ok(manifest_path)
    });

    match result {
        Ok(manifest_path) => env
            .new_string(manifest_path)
            .expect("Couldn't create java string!")
            .into_raw(),
        Err(err) => panic!("{:?}", err),
    }
}

pub fn push<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    domain: JString<'local>,
    namespace: JString<'local>,
) -> jstring {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result: Res<String> = runtime.block_on(async {
        let domain: String = env.get_string(&domain)?.into();
        let namespace_str: String = env.get_string(&namespace)?.into();
        let namespace = Namespace::try_from(namespace_str)?;

        let manifest_uri = local_domain::push_package(&domain.into(), namespace).await?;

        Ok(manifest_uri.to_string())
    });

    match result {
        Ok(manifest_path) => env
            .new_string(manifest_path)
            .expect("Couldn't create java string!")
            .into_raw(),
        Err(err) => panic!("{:?}", err),
    }
}
