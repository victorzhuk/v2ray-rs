use v2ray_rs_core::models::{
    RoutingRule, RoutingRuleSet, RuleAction, RuleMatch, ValidationError, validate_domain_keyword,
    validate_rule_match,
};

fn wildcard_rule() -> RoutingRule {
    RoutingRule {
        id: uuid::Uuid::new_v4(),
        match_condition: RuleMatch::DomainKeyword {
            keyword: "*.ru".to_string(),
        },
        action: RuleAction::Proxy,
        enabled: true,
        group: None,
        via_node: None,
    }
}

#[test]
fn wildcard_keyword_is_rejected() {
    assert_eq!(
        validate_domain_keyword("*.ru"),
        Err(ValidationError::WildcardDomainKeyword("*.ru".to_string()))
    );
    assert_eq!(
        ValidationError::WildcardDomainKeyword("*.ru".to_string()).to_string(),
        "invalid domain keyword '*.ru': a keyword is a plain substring and cannot contain '*'; use the Domain rule type for wildcard suffixes like '*.example.com'"
    );
    for keyword in ["*", "a*b", "sina*"] {
        assert!(
            matches!(
                validate_domain_keyword(keyword),
                Err(ValidationError::WildcardDomainKeyword(_))
            ),
            "keyword={keyword}"
        );
    }
}

#[test]
fn plain_keyword_is_accepted() {
    for keyword in ["sina", "sina.com", ".example"] {
        assert_eq!(validate_domain_keyword(keyword), Ok(()), "keyword={keyword}");
    }
}

#[test]
fn rule_match_dispatch_rejects_wildcard_keyword() {
    assert!(matches!(
        validate_rule_match(&RuleMatch::DomainKeyword {
            keyword: "*.ru".into()
        }),
        Err(ValidationError::WildcardDomainKeyword(_))
    ));
}

#[test]
fn validated_mutations_reject_wildcard_keyword() {
    let mut set = RoutingRuleSet::new();

    assert!(matches!(
        set.add_validated(wildcard_rule()),
        Err(ValidationError::WildcardDomainKeyword(_))
    ));
    assert!(matches!(
        set.add_at(0, wildcard_rule()),
        Err(ValidationError::WildcardDomainKeyword(_))
    ));
    assert_eq!(set.rules().len(), 0);

    let seed = RoutingRule {
        id: uuid::Uuid::new_v4(),
        match_condition: RuleMatch::GeoIp {
            country_code: "RU".into(),
        },
        action: RuleAction::Direct,
        enabled: true,
        group: None,
        via_node: None,
    };
    set.add_validated(seed).unwrap();
    let id = set.rules()[0].id;

    assert!(matches!(
        set.edit_rule(
            &id,
            Some(RuleMatch::DomainKeyword {
                keyword: "*.ru".into()
            }),
            None
        ),
        Err(ValidationError::WildcardDomainKeyword(_))
    ));
    assert_eq!(set.rules().len(), 1);
    assert_eq!(
        set.rules()[0].match_condition,
        RuleMatch::GeoIp {
            country_code: "RU".into()
        }
    );
}
