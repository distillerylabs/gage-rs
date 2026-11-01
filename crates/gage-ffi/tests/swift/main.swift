// The iOS smoke test (`make ios-smoke`): the generated bindings, linked
// against the XCFramework's static library, run against a vault
// `smoke_fixture` wrote. Exits non-zero on the first mismatch.
import Foundation

let passphrase = "correct horse battery staple"

// A prompter is called from the library's thread; the smoke is single-threaded.
final class Answers: Prompter, @unchecked Sendable {
    var attempts: [UInt32] = []
    let given: String
    init(_ given: String) { self.given = given }
    func unlock(request: UnlockRequest) throws -> UnlockResponse {
        attempts.append(request.attempt)
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
let roots = Roots(config: root + "/config", data: root + "/data", state: root + "/state")

do {
    let vault = try Vault.open(roots: roots, name: "personal")
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
    let vault = try Vault.open(roots: roots, name: "personal")
    let wrong = Answers("not the passphrase")
    _ = try vault.unlock(prompter: wrong)
    fail("a wrong passphrase unlocked the vault")
} catch GageError.LockedOrAuth {
    print("smoke: wrong passphrase is LockedOrAuth")
} catch {
    fail("wrong passphrase gave \(error)")
}

do {
    _ = try Vault.open(roots: roots, name: "no-such-vault")
    fail("an unregistered vault opened")
} catch GageError.NotFound {
    print("smoke: unknown vault is NotFound")
} catch {
    fail("unknown vault gave \(error)")
}

print("smoke: ok")
