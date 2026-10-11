use super::super::shared::{
    admin_wallet_id_from_suffix_path, admin_wallet_identity_name,
    build_admin_wallet_not_found_response, build_admin_wallet_summary_payload_with_package,
    build_admin_wallets_bad_request_response, parse_admin_wallets_limit,
    parse_admin_wallets_offset, resolve_admin_wallet_owner_summary,
    resolve_admin_wallet_user_identities,
};
use crate::handlers::admin::request::{AdminAppState, AdminRequestContext};
use crate::handlers::admin::shared::unix_secs_to_rfc3339;
use crate::GatewayError;
use aether_data::repository::wallet::stored_timestamp_unix_secs;
use axum::{
    body::Body,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(in super::super) async fn build_admin_wallet_transactions_response(
    state: &AdminAppState<'_>,
    request_context: &AdminRequestContext<'_>,
) -> Result<Response<Body>, GatewayError> {
    let Some(wallet_id) = admin_wallet_id_from_suffix_path(request_context.path(), "/transactions")
    else {
        return Ok(build_admin_wallets_bad_request_response("wallet_id 无效"));
    };
    let limit = match parse_admin_wallets_limit(request_context.query_string()) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let offset = match parse_admin_wallets_offset(request_context.query_string()) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };

    let Some(wallet) = state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::WalletId(
            &wallet_id,
        ))
        .await?
    else {
        return Ok(build_admin_wallet_not_found_response());
    };
    let owner = resolve_admin_wallet_owner_summary(state, &wallet).await?;
    let wallet_payload =
        build_admin_wallet_summary_payload_with_package(state, &wallet, &owner).await?;

    let (transactions, total) = state
        .list_admin_wallet_transactions(&wallet.id, limit, offset)
        .await?;
    let operators = resolve_admin_wallet_user_identities(
        state,
        transactions.iter().map(|transaction| {
            (
                transaction.operator_id.as_deref(),
                transaction.operator_name.as_deref(),
            )
        }),
    )
    .await?;
    let mut items = Vec::with_capacity(transactions.len());
    for transaction in transactions {
        let operator = transaction
            .operator_id
            .as_deref()
            .and_then(|id| operators.get(id));
        let operator_email = transaction
            .operator_email
            .or_else(|| operator.and_then(|(_, email)| email.clone()));
        let operator_name = admin_wallet_identity_name(
            transaction.operator_name,
            transaction.operator_id.as_deref(),
            &operators,
        );
        items.push(json!({
            "id": transaction.id,
            "wallet_id": transaction.wallet_id,
            "owner_type": owner.owner_type,
            "owner_name": owner.owner_name.clone(),
            "wallet_status": wallet.status.clone(),
            "category": transaction.category,
            "reason_code": transaction.reason_code,
            "amount": transaction.amount,
            "balance_before": transaction.balance_before,
            "balance_after": transaction.balance_after,
            "recharge_balance_before": transaction.recharge_balance_before,
            "recharge_balance_after": transaction.recharge_balance_after,
            "gift_balance_before": transaction.gift_balance_before,
            "gift_balance_after": transaction.gift_balance_after,
            "link_type": transaction.link_type,
            "link_id": transaction.link_id,
            "operator_id": transaction.operator_id,
            "operator_name": operator_name,
            "operator_email": operator_email,
            "description": transaction.description,
            "created_at": transaction
                .created_at_unix_ms
                .map(stored_timestamp_unix_secs)
                .and_then(unix_secs_to_rfc3339),
        }));
    }

    Ok(Json(json!({
        "wallet": wallet_payload,
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    }))
    .into_response())
}
