// The iOS smoke test (`make ios-smoke`): the generated bindings, linked
// against the XCFramework's static library, run against a vault
// `smoke_fixture` wrote. Exits non-zero on the first mismatch.
//
// `smoke.sh` runs one mode at a time, because an approval happens between two:
//
//   basic   opens, unlocks and lists the fixture's own vault
//   join    clones it as a new device and enrolls, leaving the code in a file
//   read    once the fixture has approved: syncs, unlocks and reads
import Foundation

let passphrase = "correct horse battery staple"

// A prompter is called from the library's thread; the smoke is single-threaded.
final class Answers: Prompter, @unchecked Sendable {
    var attempts: [UInt32] = []
    var purposes: [UnlockPurpose] = []
    let given: String
    init(_ given: String) { self.given = given }
    func unlock(request: UnlockRequest) throws -> UnlockResponse {
        attempts.append(request.attempt)
        purposes.append(request.purpose)
        // A wrong passphrase is retried until the app gives up.
        if request.attempt > 2 { throw PromptError.TooManyAttempts }
        return .passphrase(passphrase: given)
    }
    func warn(warning: Warning) {}
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("smoke: FAIL: \(message)\n".utf8))
    exit(1)
}

func expect(_ ok: Bool, _ message: @autoclosure () -> String) {
    if !ok { fail(message()) }
}

guard let root = ProcessInfo.processInfo.environment["ROOT"] else {
    fail("ROOT is not set")
}
let fixtureRoots = Roots(config: root + "/config", data: root + "/data", state: root + "/state")
let phone = root + "/phone"
let phoneRoots = Roots(config: phone + "/config", data: phone + "/data", state: phone + "/state")
let codeFile = root + "/enrollment-code"

func basic() {
    do {
        let vault = try Vault.open(roots: fixtureRoots, name: "personal")
        let good = Answers(passphrase)
        let identity = try vault.unlock(prompter: good)
        expect(good.attempts == [1], "right passphrase took attempts \(good.attempts)")
        let titles = try vault.list(identity: identity).map { $0.title }
        expect(titles == ["alpha", "bravo"], "listed \(titles)")
        print("smoke: listed \(titles)")
    } catch {
        fail("open, unlock or list: \(error)")
    }

    do {
        let vault = try Vault.open(roots: fixtureRoots, name: "personal")
        let wrong = Answers("not the passphrase")
        _ = try vault.unlock(prompter: wrong)
        fail("a wrong passphrase unlocked the vault")
    } catch GageError.LockedOrAuth {
        print("smoke: wrong passphrase is LockedOrAuth")
    } catch {
        fail("wrong passphrase gave \(error)")
    }

    do {
        _ = try Vault.open(roots: fixtureRoots, name: "no-such-vault")
        fail("an unregistered vault opened")
    } catch GageError.NotFound {
        print("smoke: unknown vault is NotFound")
    } catch {
        fail("unknown vault gave \(error)")
    }
}

func join() {
    let vault: Vault
    do {
        let cloned = try cloneVault(
            roots: phoneRoots, name: "personal", url: root + "/remote.git", device: "smoke-phone")
        expect(!cloned.holdsIdentity, "a new device claims to hold an identity")
        vault = cloned.vault
        print("smoke: cloned, and this device holds no identity")
    } catch {
        fail("clone: \(error)")
    }

    do {
        _ = try vault.unlock(prompter: Answers(passphrase))
        fail("a device with no identity unlocked the vault")
    } catch GageError.LockedOrAuth(_, let remedy) {
        guard case .enrollOrRecover = remedy else { fail("no-identity remedy was \(String(describing: remedy))") }
        print("smoke: unlocking before enrolling says to enroll")
    } catch {
        fail("unlock before enrolling gave \(error)")
    }

    do {
        let answers = Answers(passphrase)
        let request = try vault.enroll(device: "smoke-phone", ttlSeconds: nil, prompter: answers)
        expect(request.published, "the request was not published")
        expect(!request.code.isEmpty, "the code is empty")
        expect(answers.purposes == [.create], "enroll asked with purposes \(answers.purposes)")
        try request.code.write(toFile: codeFile, atomically: true, encoding: .utf8)
        print("smoke: enrolled")
    } catch {
        fail("enroll: \(error)")
    }

    do {
        try saveRemoteToken(roots: phoneRoots, host: "git.example.com", token: "tok")
        let stored = try remoteTokenHosts(roots: phoneRoots)
        expect(stored == ["git.example.com"], "stored tokens for \(stored)")
        try removeRemoteToken(roots: phoneRoots, host: "git.example.com")
        let left = try remoteTokenHosts(roots: phoneRoots)
        expect(left.isEmpty, "tokens left for \(left)")
        print("smoke: stored and removed a token")
    } catch {
        fail("remote tokens: \(error)")
    }
}

func read() {
    do {
        let vault = try Vault.open(roots: phoneRoots, name: "personal")
        let report = try vault.sync(prompter: Answers(passphrase))
        expect(report.pulled, "sync pulled nothing: \(report)")
        let identity = try vault.unlock(prompter: Answers(passphrase))
        print("smoke: synced and unlocked")

        let rows = try vault.list(identity: identity)
        let titles = rows.map { $0.title }
        expect(titles == ["alpha", "bravo"], "listed \(titles)")

        let found = try vault.search(identity: identity, pattern: "BRAVO")
        expect(found.map { $0.title } == ["bravo"], "searched \(found)")
        expect(!found[0].matchedBody, "a title match claimed the body")
        let inBody = try vault.search(identity: identity, pattern: "field-secret")
        expect(inBody.map { $0.title } == ["bravo"] && inBody[0].matchedBody, "searched \(inBody)")
        print("smoke: searched")

        let id = found[0].id
        let shown = try vault.show(identity: identity, id: id)
        expect(shown.title == "bravo", "showed \(shown.title)")
        expect(shown.fieldNames == ["user"], "field names were \(shown.fieldNames)")
        let printed = "\(shown)"
        expect(!printed.contains("secret-bravo") && !printed.contains("field-secret"), "show carried a secret")
        print("smoke: showed")

        let value = try vault.reveal(identity: identity, id: id, field: nil)
        expect(value == "secret-bravo", "revealed the wrong value")
        let field = try vault.reveal(identity: identity, id: id, field: "user")
        expect(field == "field-secret", "revealed the wrong field")
        print("smoke: revealed")

        do {
            _ = try vault.show(identity: identity, id: "00000000-0000-4000-8000-000000000000")
            fail("an unknown id was shown")
        } catch GageError.NotFound {
            print("smoke: unknown id is NotFound")
        }
    } catch {
        fail("sync, unlock or read: \(error)")
    }
}

let mode = CommandLine.arguments.dropFirst().first ?? "basic"
switch mode {
case "basic": basic()
case "join": join()
case "read": read()
default: fail("unknown mode \(mode)")
}

print("smoke: \(mode) ok")
