Feature: Complete and repair Plasma integration on supported host layouts
  Setup must use a plugin directory that the privileged installer can safely use.
  Discovery, provisioning, installation, verification and cleanup run unchanged;
  only the external KWin D-Bus service and package database are contract fixtures.

  Scenario Outline: Set up Plasma integration and remove it again
    Given Plasma uses the "<layout>" library layout
    When I complete Plasma integration setup
    Then Plasma integration is available with one installed plugin
    When I complete Plasma integration setup
    Then Plasma integration is available with one installed plugin
    When I remove Plasma integration
    Then Plasma integration and its installation receipts are removed

    Examples:
      | layout                |
      | compatibility symlink |
      | real lib64            |

  Scenario Outline: Repair integration after an update with a legacy receipt
    Given Plasma uses the "<layout>" library layout
    And a previous Plasma integration has a legacy installation receipt
    When I complete Plasma integration setup
    Then Plasma integration is available with one installed plugin
    When I remove Plasma integration
    Then Plasma integration and its installation receipts are removed

    Examples:
      | layout                |
      | compatibility symlink |
      | real lib64            |

  Scenario: An unexpected Qt directory symlink is refused
    Given Plasma uses the "redirected Qt" library layout
    When I complete Plasma integration setup
    Then Plasma setup refuses the redirected directory without changing it
