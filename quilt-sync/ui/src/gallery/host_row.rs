//! `HostRow` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Card;
use crate::kit::HostRow;

#[component]
pub fn HostRowStories() -> impl IntoView {
    let multi = RwSignal::new("analyst".to_string());
    let single = RwSignal::new("bench-scientist".to_string());
    let unknown = RwSignal::new(String::new());
    let waiting = RwSignal::new(String::new());
    let out = RwSignal::new("analyst".to_string());

    let roles = || {
        vec![
            "analyst".to_string(),
            "bench-scientist".to_string(),
            "admin".to_string(),
        ]
    };

    view! {
        <Story
            title="HostRow"
            note="Exactly one sub-line, always — a host with nothing to say still says what \
                  its role is. The switcher appears only where there is a choice: with one \
                  role the name is static text, because a select with a single option is a \
                  dead control. Signed out outranks the role, since a role means nothing \
                  without a session — and a role nobody has asked for yet outranks every \
                  reading of the role, dashed rather than dimmed so it costs no contrast."
        >
            <Cell wide=true label="several roles — switcher, and the role is not repeated">
                <Card title="Accounts">
                    <HostRow
                        host="open.quiltdata.com"
                        role=multi
                        roles=roles()
                        on_sign_in=|_| ()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="one role — static, no dead control">
                <Card title="Accounts">
                    <HostRow
                        host="custom.registry.io"
                        role=single
                        roles=vec!["bench-scientist".to_string()]
                        on_sign_in=|_| ()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="role not asked for yet — dashed, and it says so">
                <Card title="Accounts">
                    <HostRow
                        host="custom.registry.io"
                        role=waiting
                        provisional=true
                        on_sign_in=|_| ()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="role query failed — session is fine, role cannot be named">
                <Card title="Accounts">
                    <HostRow host="custom.registry.io" role=unknown on_sign_in=|_| () />
                </Card>
            </Cell>
            <Cell wide=true label="signed out — outranks the role">
                <Card title="Accounts">
                    <HostRow
                        host="custom.registry.io"
                        role=out
                        roles=roles()
                        signed_out=true
                        on_sign_in=|_| ()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="long hostname truncates">
                <Card title="Accounts">
                    <HostRow
                        host="quilt-enterprise-eu-west-1.internal.vir-biotechnology.example.com"
                        role=multi
                        roles=roles()
                        on_sign_in=|_| ()
                    />
                </Card>
            </Cell>
        </Story>
    }
}
